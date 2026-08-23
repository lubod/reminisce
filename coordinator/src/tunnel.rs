//! Reverse tunnel: TLS (or plain TCP) listener piping Android connections to the
//! registered home server over QUIC.

use std::io::BufReader;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Semaphore;
use tokio_rustls::TlsAcceptor;
use tracing::{info, warn};

use crate::types::TunnelMap;

/// Maximum concurrent tunneled client connections. Additional clients are
/// dropped (fail fast) instead of queueing unbounded tasks/memory.
pub const MAX_CONCURRENT_TUNNELS: usize = 256;

/// How long a tunnel may stay silent before it is reaped. Unlike the previous
/// fixed 1800s wall-clock kill, this resets on every received chunk, so long
/// active transfers (big uploads/downloads) are never cut off mid-flight.
const TUNNEL_IDLE_TIMEOUT: Duration = Duration::from_secs(1800);

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

/// Copies reader → writer, failing if no data arrives within `idle`. The idle
/// clock resets on every successfully transferred chunk.
async fn copy_with_idle<R, W>(mut reader: R, mut writer: W, idle: Duration) -> std::io::Result<u64>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = match tokio::time::timeout(idle, reader.read(&mut buf)).await {
            Ok(res) => res?,
            Err(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "tunnel idle timeout",
                ));
            }
        };
        if n == 0 {
            return Ok(total);
        }
        writer.write_all(&buf[..n]).await?;
        total += n as u64;
    }
}

async fn pipe<R, W>(
    client_read: R,
    client_write: W,
    quic_recv: quinn::RecvStream,
    quic_send: quinn::SendStream,
) where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let to_home = copy_with_idle(client_read, quic_send, TUNNEL_IDLE_TIMEOUT);
    let to_client = copy_with_idle(quic_recv, client_write, TUNNEL_IDLE_TIMEOUT);
    tokio::select! {
        res = to_home => {
            if let Err(e) = res { warn!("[TUNNEL] Client→home direction ended: {}", e); }
        }
        res = to_client => {
            if let Err(e) = res { warn!("[TUNNEL] Home→client direction ended: {}", e); }
        }
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

        // Bound concurrently-piped tunnels so client floods cannot spawn
        // unbounded tasks/QUIC streams on the coordinator.
        let tunnel_permits = Arc::new(Semaphore::new(MAX_CONCURRENT_TUNNELS));

        loop {
            let Ok((tcp_stream, client_addr)) = listener.accept().await else { continue };

            // Fail fast when at capacity rather than stalling the accept loop.
            let permit = match tunnel_permits.clone().try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => {
                    warn!("[TUNNEL] At capacity ({} tunnels) — dropping {}", MAX_CONCURRENT_TUNNELS, client_addr);
                    continue;
                }
            };

            let tunnels = tunnels.clone();
            let tls = tls_acceptor.clone();
            let allowed = allowed_tunnel_node_id.clone();

            tokio::spawn(async move {
                // Released when this tunnel's pipe task ends.
                let _permit = permit;
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
