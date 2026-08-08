//! Reverse tunnel: TLS (or plain TCP) listener piping Android connections to the
//! registered home server over QUIC.

use std::io::BufReader;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::TlsAcceptor;
use tracing::{info, warn};

use crate::types::TunnelMap;

pub fn load_tls_acceptor(cert: &PathBuf, key: &PathBuf) -> anyhow::Result<TlsAcceptor> {
    let cert_file = std::fs::File::open(cert)?;
    let key_file = std::fs::File::open(key)?;

    let certs: Vec<_> = rustls_pemfile::certs(&mut BufReader::new(cert_file))
        .collect::<Result<_, _>>()?;
    let private_key = rustls_pemfile::private_key(&mut BufReader::new(key_file))?
        .ok_or_else(|| anyhow::anyhow!("No private key found in {:?}", key))?;

    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, private_key)?;

    Ok(TlsAcceptor::from(Arc::new(config)))
}

async fn pipe<R, W>(
    mut client_read: R,
    mut client_write: W,
    mut quic_recv: quinn::RecvStream,
    mut quic_send: quinn::SendStream,
) where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let to_home = tokio::io::copy(&mut client_read, &mut quic_send);
    let to_client = tokio::io::copy(&mut quic_recv, &mut client_write);
    // 30-minute timeout: prevents stalled connections from holding QUIC streams
    // open, while allowing long media uploads/downloads through the tunnel.
    let timeout = tokio::time::sleep(std::time::Duration::from_secs(1800));
    tokio::select! {
        _ = to_home => {}
        _ = to_client => {}
        _ = timeout => { warn!("[TUNNEL] Pipe timed out after 1800s — closing"); }
    }
}

/// Listen on a TCP port; pipe each connection to the home server via QUIC tunnel.
/// If `tls_acceptor` is provided, terminates TLS — use with a Let's Encrypt cert.
/// When `allowed_tunnel_node_id` is set, connections are routed to exactly that node's
/// tunnel entry; otherwise (only possible with `--allow-any-tunnel`) the first entry is used.
pub fn start_tcp_tunnel_listener(
    tunnel_port: u16,
    tunnels: TunnelMap,
    tls_acceptor: Option<TlsAcceptor>,
    allowed_tunnel_node_id: Option<String>,
) {
    let tls_acceptor = tls_acceptor.map(Arc::new);

    tokio::spawn(async move {
        let addr: SocketAddr = format!("0.0.0.0:{}", tunnel_port).parse().unwrap();
        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => { warn!("[TUNNEL] Failed to bind TCP port {}: {}", tunnel_port, e); return; }
        };

        let tls_label = if tls_acceptor.is_some() { "HTTPS" } else { "HTTP" };
        info!("[TUNNEL] {} listener on :{} (Android → home server)", tls_label, tunnel_port);

        loop {
            let Ok((tcp_stream, client_addr)) = listener.accept().await else { continue };
            let tunnels = tunnels.clone();
            let tls = tls_acceptor.clone();
            let allowed = allowed_tunnel_node_id.clone();

            tokio::spawn(async move {
                let tunnel_conn = {
                    let map = tunnels.read().unwrap_or_else(|e| e.into_inner());
                    match &allowed {
                        Some(node_id) => map.get(node_id).cloned(),
                        None => map.values().next().cloned(),
                    }
                };
                let tunnel_conn = match tunnel_conn {
                    Some(c) => c,
                    None => { warn!("[TUNNEL] No home server registered — dropping {}", client_addr); return; }
                };
                let (qs, qr) = match tunnel_conn.open_bi().await {
                    Ok(s) => s,
                    Err(e) => { warn!("[TUNNEL] open_bi failed: {}", e); return; }
                };

                if let Some(acceptor) = tls {
                    match acceptor.accept(tcp_stream).await {
                        Ok(tls_stream) => {
                            let (r, w) = tokio::io::split(tls_stream);
                            pipe(r, w, qr, qs).await;
                        }
                        Err(e) => warn!("[TUNNEL] TLS handshake failed from {}: {}", client_addr, e),
                    }
                } else {
                    let (r, w) = tcp_stream.into_split();
                    pipe(r, w, qr, qs).await;
                }
            });
        }
    });
}
