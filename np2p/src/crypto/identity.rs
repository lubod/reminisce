use ed25519_dalek::{SigningKey, Signer, Verifier, VerifyingKey, Signature};
use crate::error::{Np2pError, Result};
use rcgen::{CertificateParams, DistinguishedName, KeyPair as RcKeyPair};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use std::sync::Arc;
use rustls::client::danger::{ServerCertVerifier, ServerCertVerified};
use rustls::server::danger::{ClientCertVerifier, ClientCertVerified};

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

    /// Derive a deterministic node identity from the account master secret.
    ///
    /// The same secret always yields the same node_id, so the P2P identity — and
    /// therefore the ability to authenticate to storage nodes (which pin
    /// `allowed_owner_id` to this node_id) — survives a full disk loss. It can be
    /// regenerated from the master secret the user holds, with no on-disk state.
    pub fn from_secret(secret: &str) -> Self {
        let hash = blake3::hash(format!("reminisce-p2p-identity:{}", secret).as_bytes());
        let signing_key = SigningKey::from_bytes(hash.as_bytes());
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

    /// Sign a message with this node's Ed25519 private key.
    pub fn sign(&self, msg: &[u8]) -> Vec<u8> {
        self.signing_key.sign(msg).to_bytes().to_vec()
    }

    /// Create a signed ShardToken for a given shard hash.
    pub fn create_shard_token(&self, shard_hash: &[u8; 32]) -> ShardToken {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or(std::time::Duration::from_secs(0))
            .as_secs();
        let mut msg_to_sign = Vec::new();
        msg_to_sign.extend_from_slice(shard_hash);
        msg_to_sign.extend_from_slice(&timestamp.to_be_bytes());
        let signature = self.sign(&msg_to_sign);
        ShardToken {
            owner_node_id: self.node_id(),
            timestamp,
            signature,
        }
    }

    pub fn generate_tls_config(&self) -> Result<(quinn::ServerConfig, quinn::ClientConfig)> {
        // Install default crypto provider for rustls 0.23+
        let _ = rustls::crypto::ring::default_provider().install_default();

        let node_id_hex = hex::encode(self.node_id());
        let mut params = CertificateParams::default();
        params.distinguished_name = DistinguishedName::new();
        params.distinguished_name.push(rcgen::DnType::CommonName, format!("np2p-node-{}", node_id_hex));
        params.subject_alt_names = vec![rcgen::SanType::DnsName(node_id_hex.clone().try_into().unwrap())];

        // Format raw secret key as PKCS#8 DER (48 bytes total)
        let secret_bytes = self.signing_key.to_bytes();
        let mut pkcs8 = Vec::with_capacity(48);
        pkcs8.extend_from_slice(&[
            0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20
        ]);
        pkcs8.extend_from_slice(&secret_bytes);

        let private_key_der = rustls_pki_types::PrivatePkcs8KeyDer::from(pkcs8);
        let rc_keypair = RcKeyPair::from_pkcs8_der_and_sign_algo(&private_key_der, &rcgen::PKCS_ED25519)
            .map_err(|e| Np2pError::Crypto(format!("Failed to load rcgen keypair: {}", e)))?;

        let cert = params.self_signed(&rc_keypair)
            .map_err(|e| Np2pError::Crypto(format!("Failed to generate cert: {}", e)))?;
        
        let cert_der = cert.der().clone();
        let key_der = rc_keypair.serialize_der();

        let cert_chain = vec![cert_der];
        let private_key = PrivateKeyDer::Pkcs8(key_der.into());

        let mut server_config = rustls::ServerConfig::builder()
            .with_client_cert_verifier(Arc::new(AcceptAnyClientCert))
            .with_single_cert(cert_chain.clone(), private_key.clone_key())
            .map_err(|e| Np2pError::Crypto(format!("TLS config error: {}", e)))?;
        server_config.alpn_protocols = vec![b"np2p".to_vec()];

        let mut client_config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(VerifyNodeCertificate))
            .with_client_auth_cert(cert_chain, private_key)
            .map_err(|e| Np2pError::Crypto(format!("TLS client cert error: {}", e)))?;
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

/// Verify an Ed25519 signature produced by the node whose public key is `node_id_bytes` over `msg`.
pub fn verify_signature(node_id_bytes: &[u8], msg: &[u8], signature_bytes: &[u8]) -> bool {
    let Ok(key_bytes) = <[u8; 32]>::try_from(node_id_bytes) else { return false };
    let Ok(sig_bytes) = <[u8; 64]>::try_from(signature_bytes) else { return false };
    let Ok(key) = VerifyingKey::from_bytes(&key_bytes) else { return false };
    let sig = Signature::from_bytes(&sig_bytes);
    key.verify(msg, &sig).is_ok()
}

/// Extract Ed25519 public key bytes from raw DER encoded X.509 certificate.
pub fn extract_public_key(cert_der: &[u8]) -> Option<[u8; 32]> {
    let oid = [0x06, 0x03, 0x2b, 0x65, 0x70]; // Ed25519 OID
    for pos in 0..cert_der.len().saturating_sub(oid.len() + 3 + 32) {
        if cert_der[pos..pos+oid.len()] == oid
           && cert_der[pos + oid.len()] == 0x03 // BIT STRING
           && cert_der[pos + oid.len() + 1] == 0x21 // Length 33
           && cert_der[pos + oid.len() + 2] == 0x00 // Unused bits 0
        {
            let mut key = [0u8; 32];
            key.copy_from_slice(&cert_der[pos + oid.len() + 3 .. pos + oid.len() + 35]);
            return Some(key);
        }
    }
    None
}

/// A verifier that verifies Node self-signed certificates against the expected Node ID.
#[derive(Debug)]
struct VerifyNodeCertificate;

impl ServerCertVerifier for VerifyNodeCertificate {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let cert_bytes = end_entity.as_ref();
        let pubkey = extract_public_key(cert_bytes)
            .ok_or_else(|| rustls::Error::General("Invalid or missing Ed25519 public key in certificate".into()))?;

        // If the server_name is a hex-encoded Node ID (64 chars), verify it matches the public key
        if let ServerName::DnsName(dns_name) = server_name {
            let name_str = dns_name.as_ref();
            if name_str.len() == 64 && name_str.chars().all(|c| c.is_ascii_hexdigit()) {
                let expected_key_bytes = hex::decode(name_str)
                    .map_err(|_| rustls::Error::General("Failed to decode server name as hex".into()))?;
                if pubkey != expected_key_bytes.as_slice() {
                    return Err(rustls::Error::General("Certificate public key does not match expected Node ID".into()));
                }
            }
        }
        
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![rustls::SignatureScheme::ED25519]
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone, PartialEq, Eq)]
pub struct ShardToken {
    pub owner_node_id: [u8; 32],
    pub timestamp: u64,
    pub signature: Vec<u8>,
}

pub fn verify_shard_token(
    token: &ShardToken,
    shard_hash: &[u8; 32],
    allowed_owner_id: Option<&[u8; 32]>,
) -> bool {
    // 1. Check if owner_node_id is allowed
    if let Some(allowed) = allowed_owner_id {
        if &token.owner_node_id != allowed {
            return false;
        }
    }

    // 2. Check if timestamp is recent (prevent replay attacks, 5 minutes window)
    let now = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or(std::time::Duration::from_secs(0))
        .as_secs();
    if now.saturating_sub(token.timestamp) > 300 && token.timestamp.saturating_sub(now) > 300 {
        return false;
    }

    // 3. Verify Ed25519 signature
    let mut msg_to_sign = Vec::new();
    msg_to_sign.extend_from_slice(shard_hash);
    msg_to_sign.extend_from_slice(&token.timestamp.to_be_bytes());
    
    verify_signature(&token.owner_node_id, &msg_to_sign, &token.signature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_secret_is_deterministic() {
        let a = NodeIdentity::from_secret("my-master-secret");
        let b = NodeIdentity::from_secret("my-master-secret");
        assert_eq!(a.node_id(), b.node_id(), "same secret must yield same node_id");

        let c = NodeIdentity::from_secret("different-secret");
        assert_ne!(a.node_id(), c.node_id(), "different secret must yield different node_id");

        // A random identity is (overwhelmingly) not the derived one.
        let r = NodeIdentity::generate();
        assert_ne!(a.node_id(), r.node_id());
    }

    #[test]
    fn test_print_cert_der() {
        let identity = NodeIdentity::generate();
        let _ = identity.generate_tls_config().unwrap();
        let node_id_hex = hex::encode(identity.node_id());
        let mut params = CertificateParams::default();
        params.distinguished_name = DistinguishedName::new();
        params.distinguished_name.push(rcgen::DnType::CommonName, format!("np2p-node-{}", node_id_hex));
        params.subject_alt_names = vec![rcgen::SanType::DnsName(node_id_hex.clone().try_into().unwrap())];

        let secret_bytes = identity.signing_key.to_bytes();
        let mut pkcs8 = Vec::with_capacity(48);
        pkcs8.extend_from_slice(&[
            0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20
        ]);
        pkcs8.extend_from_slice(&secret_bytes);

        let private_key_der = rustls_pki_types::PrivatePkcs8KeyDer::from(pkcs8);
        let rc_keypair = RcKeyPair::from_pkcs8_der_and_sign_algo(&private_key_der, &rcgen::PKCS_ED25519).unwrap();
        let cert = params.self_signed(&rc_keypair).unwrap();
        let cert_der = cert.der();

        let extracted = extract_public_key(cert_der);
        println!("EXTRACTED: {:?}", extracted.map(hex::encode));
        println!("EXPECTED : {}", hex::encode(identity.node_id()));
        assert_eq!(extracted.unwrap(), identity.node_id());
    }
}

/// A verifier that requests and accepts any client certificate (so conn.peer_identity() works).
#[derive(Debug)]
struct AcceptAnyClientCert;

impl ClientCertVerifier for AcceptAnyClientCert {
    fn root_hint_subjects(&self) -> &[rustls::DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> std::result::Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![rustls::SignatureScheme::ED25519]
    }
}
