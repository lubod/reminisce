use ed25519_dalek::SigningKey;
use crate::error::{Np2pError, Result};
use rcgen::{CertificateParams, DistinguishedName, KeyPair as RcKeyPair};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use std::sync::Arc;
use rustls::client::danger::{ServerCertVerifier, ServerCertVerified};

pub const NODE_ID_LENGTH: usize = 32;

pub struct NodeIdentity {
    pub signing_key: SigningKey,
}

impl Clone for NodeIdentity {
    fn clone(&self) -> Self {
        Self {
            signing_key: SigningKey::from_bytes(&self.signing_key.to_bytes()),
        }
    }
}

impl NodeIdentity {
    pub fn generate() -> Self {
        let mut rng = rand::thread_rng();
        let signing_key = SigningKey::generate(&mut rng);
        Self { signing_key }
    }

    pub fn from_secret_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != 32 {
            return Err(Np2pError::Identity(format!("Invalid secret key size: expected 32, got {}", bytes.len())));
        }
        let array: [u8; 32] = bytes.try_into().map_err(|_| Np2pError::Identity("Failed to convert key bytes".into()))?;
        let signing_key = SigningKey::from_bytes(&array);
        Ok(Self { signing_key })
    }

    pub fn node_id(&self) -> [u8; NODE_ID_LENGTH] {
        self.signing_key.verifying_key().to_bytes()
    }

    pub fn generate_tls_config(&self) -> Result<(quinn::ServerConfig, quinn::ClientConfig)> {
        // Install default crypto provider for rustls 0.23+
        let _ = rustls::crypto::ring::default_provider().install_default();

        let node_id_hex = hex::encode(self.node_id());
        let mut params = CertificateParams::default();
        params.distinguished_name = DistinguishedName::new();
        params.distinguished_name.push(rcgen::DnType::CommonName, format!("np2p-node-{}", node_id_hex));
        params.subject_alt_names = vec![rcgen::SanType::DnsName(node_id_hex.clone().try_into().unwrap())];

        let rc_keypair = RcKeyPair::generate_for(&rcgen::PKCS_ED25519)
            .map_err(|e| Np2pError::Crypto(format!("Failed to create rcgen keypair: {}", e)))?;

        let cert = params.self_signed(&rc_keypair)
            .map_err(|e| Np2pError::Crypto(format!("Failed to generate cert: {}", e)))?;
        
        let cert_der = cert.der().clone();
        let key_der = rc_keypair.serialize_der();

        let cert_chain = vec![cert_der];
        let private_key = PrivateKeyDer::Pkcs8(key_der.into());

        let mut server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(cert_chain.clone(), private_key)
            .map_err(|e| Np2pError::Crypto(format!("TLS config error: {}", e)))?;
        server_config.alpn_protocols = vec![b"np2p".to_vec()];

        let mut client_config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
            .with_no_client_auth();
        client_config.alpn_protocols = vec![b"np2p".to_vec()];

        // Convert to QUIC configs
        let server_quic = quinn::crypto::rustls::QuicServerConfig::try_from(server_config)
            .map_err(|e| Np2pError::Crypto(format!("QUIC server config error: {}", e)))?;
        let client_quic = quinn::crypto::rustls::QuicClientConfig::try_from(client_config)
            .map_err(|e| Np2pError::Crypto(format!("QUIC client config error: {}", e)))?;

        Ok((
            quinn::ServerConfig::with_crypto(Arc::new(server_quic)),
            quinn::ClientConfig::new(Arc::new(client_quic))
        ))
    }
}

/// A verifier that skips standard CA verification but could be used to verify Node IDs.
#[derive(Debug)]
struct SkipServerVerification;

impl ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![rustls::SignatureScheme::ED25519]
    }
}
