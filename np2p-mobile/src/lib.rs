use std::sync::Arc;
use tokio::runtime::Runtime;
use once_cell::sync::Lazy;
use np2p::network::P2PService;
use np2p::crypto::NodeIdentity;
use np2p::network::Message;
use np2p::network::protocol::Protocol;

uniffi::setup_scaffolding!();

static RUNTIME: Lazy<Runtime> = Lazy::new(|| {
    // Initialize tracing to Android logcat
    #[cfg(target_os = "android")]
    {
        use tracing_subscriber::prelude::*;
        let android_layer = tracing_android::layer("NP2P").unwrap();
        tracing_subscriber::registry()
            .with(android_layer)
            .init();
    }

    // Install default crypto provider for rustls 0.23+
    let _ = rustls::crypto::ring::default_provider().install_default();

    Runtime::new().expect("Failed to create Tokio runtime")
});

#[uniffi::export]
pub fn init_logging() {
    // This triggers the Lazy RUNTIME initialization which starts the logger
    let _ = &*RUNTIME;
}

#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum MobileP2pError {
    #[error("Network error: {0}")]
    Network(String),
    #[error("Authentication failed: {0}")]
    Auth(String),
    #[error("Internal error: {0}")]
    Internal(String),
}

#[derive(uniffi::Object)]
pub struct MobileClient {
    runtime: &'static Runtime,
}

#[uniffi::export]
impl MobileClient {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            runtime: &RUNTIME,
        })
    }

    pub fn upload_media(
        &self,
        server_target: String, // IP address of the server
        jwt_token: String,
        device_id: String,
        file_path: String,
        file_hash: String,
        file_name: String,
        file_ext: String,
    ) -> Result<bool, MobileP2pError> {
        self.runtime.block_on(async move {
            // Generate ephemeral identity for this session
            let identity = NodeIdentity::generate();

            // Initialize service
            let service = P2PService::new(
                "0.0.0.0:0".parse().unwrap(),
                identity
            ).await.map_err(|e| MobileP2pError::Network(e.to_string()))?;

            // Connect to server via direct IP
            let addr = server_target.parse::<std::net::SocketAddr>()
                .map_err(|e| MobileP2pError::Internal(format!("Invalid server address: {}", e)))?;
            let conn = service.connect_to_addr(addr).await
                .map_err(|e| MobileP2pError::Network(format!("Direct connection to {} failed: {}", addr, e)))?;

            // Authenticate via JWT
            let (mut send, mut recv) = conn.open_bi().await
                .map_err(|e| MobileP2pError::Network(e.to_string()))?;

            Protocol::send(&mut send, &Message::Authenticate { token: jwt_token }).await
                .map_err(|e| MobileP2pError::Internal(e.to_string()))?;
            send.finish().ok();

            let auth_resp = Protocol::receive(&mut recv).await
                .map_err(|e| MobileP2pError::Internal(e.to_string()))?;

            match auth_resp {
                Message::AuthenticateResponse { success, message } => {
                    if !success {
                        return Err(MobileP2pError::Auth(message));
                    }
                }
                _ => return Err(MobileP2pError::Internal("Unexpected auth response".into())),
            }

            // Read file data
            let data = tokio::fs::read(&file_path).await
                .map_err(|e| MobileP2pError::Internal(format!("File read error: {}", e)))?;

            // Send media
            let (mut upload_send, mut upload_recv) = conn.open_bi().await
                .map_err(|e| MobileP2pError::Network(e.to_string()))?;

            Protocol::send(&mut upload_send, &Message::UploadMediaRequest {
                device_id,
                file_hash,
                file_name,
                file_ext,
                data,
            }).await.map_err(|e| MobileP2pError::Internal(e.to_string()))?;
            upload_send.finish().ok();

            let upload_resp = Protocol::receive(&mut upload_recv).await
                .map_err(|e| MobileP2pError::Internal(e.to_string()))?;

            match upload_resp {
                Message::UploadMediaResponse { success, message } => {
                    if success {
                        Ok(true)
                    } else {
                        Err(MobileP2pError::Internal(message))
                    }
                }
                _ => Err(MobileP2pError::Internal("Unexpected upload response".into())),
            }
        })
    }

    pub fn p2p_login(
        &self,
        server_target: String, // IP address of the server
        username: String,
        password_hash: String,
    ) -> Result<String, MobileP2pError> {
        self.runtime.block_on(async move {
            // Identity
            let identity = NodeIdentity::generate();

            // Service
            let service = P2PService::new(
                "0.0.0.0:0".parse().unwrap(),
                identity
            ).await.map_err(|e| MobileP2pError::Network(e.to_string()))?;

            // Connect via direct IP
            let addr = server_target.parse::<std::net::SocketAddr>()
                .map_err(|e| MobileP2pError::Internal(format!("Invalid server address: {}", e)))?;
            let conn = service.connect_to_addr(addr).await
                .map_err(|e| MobileP2pError::Network(format!("Direct connection to {} failed: {}", addr, e)))?;

            // Login Request
            let (mut send, mut recv) = conn.open_bi().await
                .map_err(|e| MobileP2pError::Network(e.to_string()))?;

            Protocol::send(&mut send, &Message::LoginRequest { username, password_hash }).await
                .map_err(|e| MobileP2pError::Internal(e.to_string()))?;
            send.finish().ok();

            let resp = Protocol::receive(&mut recv).await
                .map_err(|e| MobileP2pError::Internal(e.to_string()))?;

            match resp {
                Message::LoginResponse { success, token, message } => {
                    if success {
                        token.ok_or_else(|| MobileP2pError::Auth("No token returned".into()))
                    } else {
                        Err(MobileP2pError::Auth(message))
                    }
                }
                _ => Err(MobileP2pError::Internal("Unexpected login response".into())),
            }
        })
    }
}
