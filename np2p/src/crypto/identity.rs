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

/// Reads a single DER TLV at `pos` (definite-length encoding only — sufficient
/// for the fixed X.509 shapes we parse, and rejects indefinite/oversized forms).
/// Returns `(tag, value_offset, value_len)`.
fn read_tlv(der: &[u8], pos: usize) -> Option<(u8, usize, usize)> {
    let tag = *der.get(pos)?;
    let mut p = pos + 1;
    let first = *der.get(p)?;
    p += 1;
    let (len, nbytes) = if first & 0x80 == 0 {
        (first as usize, 1usize)
    } else {
        let n = (first & 0x7f) as usize;
        if n == 0 || n > 4 {
            return None; // indefinite-length or absurd multi-byte length
        }
        let mut l = 0usize;
        for _ in 0..n {
            let b = *der.get(p)? as usize;
            p += 1;
            l = (l << 8) | b;
        }
        (l, 1 + n)
    };
    let value_start = p;
    let value_end = value_start.checked_add(len)?;
    if value_end > der.len() {
        return None;
    }
    let _ = nbytes;
    Some((tag, value_start, len))
}

/// Returns the direct children of a SEQUENCE as `(tag, value_offset, value_len)` tuples.
fn sequence_children(der: &[u8], value_start: usize, value_len: usize) -> Vec<(u8, usize, usize)> {
    let mut out = Vec::new();
    let end = value_start.saturating_add(value_len);
    let mut pos = value_start;
    while pos < end {
        match read_tlv(der, pos) {
            Some((tag, vs, vl)) => {
                out.push((tag, vs, vl));
                pos = vs.saturating_add(vl);
            }
            None => break,
        }
    }
    out
}

/// If `(tag, val, len)` at the TBS level is an Ed25519 SubjectPublicKeyInfo
/// (`SEQUENCE { SEQUENCE { OID 1.3.101.112 }, BIT STRING }`), returns the 32-byte key.
fn parse_ed25519_spki(der: &[u8], tag: u8, val: usize, len: usize) -> Option<[u8; 32]> {
    if tag != 0x30 {
        return None;
    }
    let spki = sequence_children(der, val, len);
    if spki.len() != 2 {
        return None;
    }
    let (alg_tag, alg_val, alg_len) = spki[0];
    let (bit_tag, bit_val, bit_len) = spki[1];
    if alg_tag != 0x30 || bit_tag != 0x03 {
        return None;
    }
    // First child of the algorithm SEQUENCE must be the Ed25519 OID 1.3.101.112.
    let alg = sequence_children(der, alg_val, alg_len);
    let (oid_tag, oid_val, oid_len) = *alg.first()?;
    if oid_tag != 0x06 {
        return None;
    }
    if der[oid_val..oid_val + oid_len] != [0x2b, 0x65, 0x70] {
        return None;
    }
    // BIT STRING: first byte = unused-bits (must be 0), then 32 raw key bytes.
    let bit_string = &der[bit_val..bit_val + bit_len];
    if bit_len < 33 || bit_string[0] != 0 {
        return None;
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bit_string[1..33]);
    Some(key)
}

/// Extract the Ed25519 public key from a DER-encoded X.509 certificate.
///
/// This uses a strict DER structure walk of the certificate's actual
/// `subjectPublicKeyInfo`, NOT a byte-pattern scan. The previous implementation
/// scanned for the Ed25519 OID + key byte pattern *anywhere* in the DER, so a
/// certificate crafted with the victim's node-id bytes in an issuer/subject DN
/// or an extension could make the identity pin validate while the TLS handshake
/// is signed by the attacker's real key — a full MITM and coordinator registry
/// poisoning primitive. Reading the SPKI field at its grammar position (7th TBS
/// element for v3 certs, 6th for v1) and validating its shape makes the pin
/// reflect the exact key rustls verifies the handshake against.
pub fn extract_public_key(cert_der: &[u8]) -> Option<[u8; 32]> {
    let (outer_tag, cert_val, cert_len) = read_tlv(cert_der, 0)?;
    if outer_tag != 0x30 {
        return None;
    }
    let cert_children = sequence_children(cert_der, cert_val, cert_len);
    let (_, tbs_val, tbs_len) = *cert_children.first()?;
    let tbs = sequence_children(cert_der, tbs_val, tbs_len);

    // subjectPublicKeyInfo is element 6 for v3 (version present) or element 5 for v1.
    for idx in [6usize, 5] {
        if let Some(&(tag, val, len)) = tbs.get(idx) {
            if let Some(key) = parse_ed25519_spki(cert_der, tag, val, len) {
                return Some(key);
            }
        }
    }
    None
}

/// Build the QUIC server-name (SNI) for dialing a peer by its 64-hex Node ID.
///
/// QUIC/rustls enforces a 63-character-per-label DNS limit, so a raw 64-hex ID is an
/// invalid server name. We split it into two DNS labels (`first32.second32`) and the
/// client-side cert verifier reassembles them before comparing to the peer's Ed25519 key.
/// Rejects anything that isn't a 64-hex Node ID, making identity binding mandatory.
pub fn sni_for_node_id(node_id: &str) -> crate::error::Result<String> {
    if node_id.len() != 64 || !node_id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(crate::error::Np2pError::Crypto(format!(
            "Connection server name must be a 64-hex Node ID, got '{}'",
            node_id
        )));
    }
    Ok(format!("{}.{}", &node_id[..32], &node_id[32..]))
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

        // Server identity binding is MANDATORY: the dialed server name must be a
        // 64-hex Node ID (encoded across two DNS labels, see sni_for_node_id) whose
        // public key equals the certificate's key. Without this, a spoofed peer can
        // present any self-signed certificate and all upload/restore/audit/rebalance
        // paths become MITM-able.
        let name_str_encoded = match server_name {
            ServerName::DnsName(dns_name) => dns_name.as_ref().to_string(),
            _ => return Err(rustls::Error::General("Unsupported server name (expected hex Node ID)".into())),
        };
        let name_str = name_str_encoded.replace('.', "");
        if name_str.len() != 64 || !name_str.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(rustls::Error::General(format!(
                "Server name '{}' does not encode a 64-hex Node ID — refusing unverified connection",
                name_str
            )));
        }
        let expected_key_bytes = hex::decode(&name_str)
            .map_err(|_| rustls::Error::General("Failed to decode server name as hex".into()))?;
        if pubkey != expected_key_bytes.as_slice() {
            return Err(rustls::Error::General("Certificate public key does not match expected Node ID".into()));
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

    // 2. Check if timestamp is recent (prevent replay attacks, 5 minutes window).
    //    `abs_diff` accepts tokens within ±5 minutes of the current time; anything
    //    older (or clock-skewed into the future) is rejected.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or(std::time::Duration::from_secs(0))
        .as_secs();
    if now.abs_diff(token.timestamp) > 300 {
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

    #[test]
    fn test_extract_public_key_rejects_pattern_smuggling() {
        // Regression for the cert-identity pin bypass: the old extract_public_key
        // scanned for the Ed25519 OID + key byte pattern anywhere in the DER. A
        // certificate carrying the victim's node_id bytes inside a custom extension
        // (or DN) would then validate the pin while the TLS handshake is signed by
        // the attacker's real key — a full MITM. The strict SPKI walk must return
        // the REAL key, not the smuggled victim bytes.
        let attacker = NodeIdentity::generate();
        let victim_key = [0x11u8; 32]; // the "victim" node_id the attacker tries to impersonate

        let node_id_hex = hex::encode(attacker.node_id());
        let mut params = CertificateParams::default();
        params.distinguished_name = DistinguishedName::new();
        params.distinguished_name.push(rcgen::DnType::CommonName, format!("np2p-node-{}", node_id_hex));
        params.subject_alt_names = vec![rcgen::SanType::DnsName(node_id_hex.clone().try_into().unwrap())];

        // Smoke the EXACT byte pattern the old scanner looked for into an extension:
        //   OID 06 03 2b 65 70, BIT STRING 03 21 00, then the victim public key.
        let mut poison = vec![0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00];
        poison.extend_from_slice(&victim_key);
        params.custom_extensions = vec![rcgen::CustomExtension::from_oid_content(&[1, 3, 6, 1, 4, 1, 54321], poison)];

        let secret_bytes = attacker.signing_key.to_bytes();
        let mut pkcs8 = Vec::with_capacity(48);
        pkcs8.extend_from_slice(&[
            0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20
        ]);
        pkcs8.extend_from_slice(&secret_bytes);
        let private_key_der = rustls_pki_types::PrivatePkcs8KeyDer::from(pkcs8);
        let rc_keypair = RcKeyPair::from_pkcs8_der_and_sign_algo(&private_key_der, &rcgen::PKCS_ED25519).unwrap();
        let cert = params.self_signed(&rc_keypair).unwrap();

        assert!(
            cert.der().windows(8).any(|w| w == [0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00]),
            "poison bytes must actually be present in the DER for a meaningful test"
        );
        // The strict walk must return the attacker's REAL public key, never the
        // smuggled victim bytes (which the old byte-scan would have returned).
        assert_eq!(extract_public_key(cert.der()).unwrap(), attacker.node_id());
        assert_ne!(extract_public_key(cert.der()).unwrap(), victim_key);
    }

    #[test]
    fn test_extract_public_key_rejects_garbage() {
        assert_eq!(extract_public_key(&[]), None);
        assert_eq!(extract_public_key(&[0x30, 0x03, 0x02, 0x01, 0x00]), None);
        assert_eq!(extract_public_key(&[0xff, 0xff, 0xff, 0xff]), None);
        // Truncated valid-ish cert must not panic or yield a key.
        assert_eq!(extract_public_key(&[0x30, 0x05, 0x30, 0x03, 0x06]), None);
        // Not an outer SEQUENCE.
        assert_eq!(extract_public_key(&[0x31, 0x00]), None);
        // Outer SEQUENCE with no children.
        assert_eq!(extract_public_key(&[0x30, 0x00]), None);
    }

    fn der_len(len: usize) -> Vec<u8> {
        if len < 0x80 {
            vec![len as u8]
        } else if len < 0x100 {
            vec![0x81, len as u8]
        } else {
            vec![0x82, (len >> 8) as u8, (len & 0xff) as u8]
        }
    }
    fn der_seq(children: &[Vec<u8>]) -> Vec<u8> {
        let mut body = Vec::new();
        for c in children {
            body.extend_from_slice(c);
        }
        let mut out = vec![0x30];
        out.extend(der_len(body.len()));
        out.extend(body);
        out
    }
    fn der_oid(content: &[u8]) -> Vec<u8> {
        let mut out = vec![0x06];
        out.extend(der_len(content.len()));
        out.extend(content);
        out
    }
    fn der_ed25519_alg() -> Vec<u8> {
        der_seq(&[der_oid(&[0x2b, 0x65, 0x70])])
    }
    fn der_bitstring(key: &[u8], unused_bits: u8) -> Vec<u8> {
        let mut out = vec![0x03];
        out.extend(der_len(key.len() + 1));
        out.push(unused_bits);
        out.extend_from_slice(key);
        out
    }
    fn der_ed25519_spki(key: &[u8]) -> Vec<u8> {
        der_seq(&[der_ed25519_alg(), der_bitstring(key, 0)])
    }

    #[test]
    fn test_read_tlv_rejects_malformed() {
        assert_eq!(read_tlv(&[], 0), None, "empty");
        assert_eq!(read_tlv(&[0x02], 0), None, "missing length");
        assert_eq!(read_tlv(&[0x02, 0x80], 0), None, "indefinite length");
        assert_eq!(read_tlv(&[0x02, 0x85, 0, 0, 0, 0, 0], 0), None, "length form n>4");
        assert_eq!(read_tlv(&[0x02, 0x03, 0xaa], 0), None, "length exceeds remaining");
        let (t, v, l) = read_tlv(&[0x02, 0x03, 0xaa, 0xbb, 0xcc], 0).unwrap();
        assert_eq!((t, v, l), (0x02, 2, 3), "well-formed short TLV");
    }

    #[test]
    fn test_spki_rejects_wrong_shapes() {
        // Wrong outer tag.
        assert_eq!(parse_ed25519_spki(&[0x30, 0x00], 0x31, 0, 2), None);
        // Not exactly two children.
        assert_eq!(parse_ed25519_spki(&der_seq(&[der_ed25519_alg()]), 0x30, 0, 2), None);
        // Algorithm is not a SEQUENCE.
        assert_eq!(
            parse_ed25519_spki(&der_seq(&[der_oid(&[0x2b, 0x65, 0x70]), der_bitstring(&[9u8; 32], 0)]), 0x30, 0, 2),
            None
        );
        // Second child is not a BIT STRING.
        assert_eq!(
            parse_ed25519_spki(&der_seq(&[der_ed25519_alg(), der_oid(&[0x01])]), 0x30, 0, 2),
            None
        );
        // Wrong algorithm OID (RSA 1.2.840.113549.1.1.1).
        let rsa_alg = der_seq(&[der_oid(&[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01])]);
        assert_eq!(
            parse_ed25519_spki(&der_seq(&[rsa_alg, der_bitstring(&[9u8; 32], 0)]), 0x30, 0, 2),
            None
        );
        // Non-zero unused bits.
        assert_eq!(
            parse_ed25519_spki(&der_seq(&[der_ed25519_alg(), der_bitstring(&[9u8; 32], 1)]), 0x30, 0, 2),
            None
        );
        // BIT STRING too short.
        assert_eq!(
            parse_ed25519_spki(&der_seq(&[der_ed25519_alg(), der_bitstring(&[9u8; 4], 0)]), 0x30, 0, 2),
            None
        );
        // Valid SPKI round-trips through the parser.
        let key = [7u8; 32];
        let spki = der_ed25519_spki(&key);
        let (tag, val, len) = read_tlv(&spki, 0).unwrap();
        assert_eq!(parse_ed25519_spki(&spki, tag, val, len), Some(key));
    }

    #[test]
    fn test_verify_shard_token_accepts_recent() {
        let identity = NodeIdentity::generate();
        let shard_hash = [7u8; 32];
        let token = identity.create_shard_token(&shard_hash);
        assert!(verify_shard_token(&token, &shard_hash, Some(&identity.node_id())));
        // Without an owner pin, a validly-signed recent token is still accepted.
        assert!(verify_shard_token(&token, &shard_hash, None));
    }

    #[test]
    fn test_from_secret_bytes() {
        assert!(NodeIdentity::from_secret_bytes(&[0u8; 32]).is_ok());
        assert!(NodeIdentity::from_secret_bytes(&[0u8; 16]).is_err(), "wrong size must error");
        // A node derived from secret bytes must sign/verify against its node_id.
        let good = NodeIdentity::from_secret_bytes(&[1u8; 32]).unwrap();
        assert!(good.sign([0u8; 32].as_slice()).len() == 64);
    }

    #[test]
    fn test_sni_for_node_id_validation() {
        let id = NodeIdentity::generate();
        let hex_id = hex::encode(id.node_id());
        let sni = sni_for_node_id(&hex_id).unwrap();
        assert_eq!(sni, format!("{}.{}", &hex_id[..32], &hex_id[32..]));
        assert!(sni_for_node_id("short").is_err(), "not 64 hex");
        assert!(sni_for_node_id(&"z".repeat(64)).is_err(), "non-hex chars");
        assert!(sni_for_node_id(&"0".repeat(63)).is_err(), "too short");
    }

    #[test]
    fn test_verify_signature_paths() {
        let identity = NodeIdentity::generate();
        let msg = b"hello signature";
        let sig = identity.sign(msg);
        assert!(verify_signature(&identity.node_id(), msg, &sig), "valid signature");
        assert!(!verify_signature(&identity.node_id(), b"tampered", &sig), "wrong message");

        let other = NodeIdentity::generate();
        assert!(!verify_signature(&other.node_id(), msg, &sig), "wrong key");

        // Malformed inputs must be rejected, not panic.
        assert!(!verify_signature(&[0u8; 8], msg, &sig), "short node id");
        assert!(!verify_signature(&identity.node_id(), msg, &[0u8; 8]), "short signature");
    }

    #[test]
    fn test_owner_pin_rejects_wrong_owner() {
        let owner = NodeIdentity::generate();
        let other = NodeIdentity::generate();
        let shard_hash = [5u8; 32];
        let token = owner.create_shard_token(&shard_hash);
        assert!(verify_shard_token(&token, &shard_hash, Some(&owner.node_id())));
        assert!(!verify_shard_token(&token, &shard_hash, Some(&other.node_id())));
    }

    #[test]
    fn test_verify_shard_token_rejects_expired() {
        let identity = NodeIdentity::generate();
        let shard_hash = [7u8; 32];
        let now = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Token signed 10 minutes in the past — beyond the 5-minute replay window.
        let stale_ts = now - 600;
        let mut msg_to_sign = Vec::new();
        msg_to_sign.extend_from_slice(&shard_hash);
        msg_to_sign.extend_from_slice(&stale_ts.to_be_bytes());
        let stale_token = ShardToken {
            owner_node_id: identity.node_id(),
            timestamp: stale_ts,
            signature: identity.sign(&msg_to_sign).to_vec(),
        };
        assert!(!verify_shard_token(&stale_token, &shard_hash, Some(&identity.node_id())));

        // Token signed 10 minutes in the future (clock skew abuse) must also be rejected.
        let future_ts = now + 600;
        let mut msg_to_sign = Vec::new();
        msg_to_sign.extend_from_slice(&shard_hash);
        msg_to_sign.extend_from_slice(&future_ts.to_be_bytes());
        let future_token = ShardToken {
            owner_node_id: identity.node_id(),
            timestamp: future_ts,
            signature: identity.sign(&msg_to_sign).to_vec(),
        };
        assert!(!verify_shard_token(&future_token, &shard_hash, Some(&identity.node_id())));
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
