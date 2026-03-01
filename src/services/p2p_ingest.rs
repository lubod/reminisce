use async_trait::async_trait;
use np2p::network::handler::{P2PHandler};
use np2p::network::Message;
use np2p::error::Result as Np2pResult;
use deadpool_postgres::Pool;
use crate::config::Config;
use std::sync::Arc;
use tracing::{info, error, warn};
use std::path::PathBuf;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Validation, Algorithm, Header};
use crate::Claims;
use tokio::sync::Mutex;
use crate::auth_utils::verify_password;
use uuid::Uuid;

pub struct IngestHandler {
    pool: Pool,
    config: Arc<Config>,
    authenticated: Mutex<bool>,
}

impl IngestHandler {
    pub fn new(pool: Pool, config: Arc<Config>) -> Self {
        Self { 
            pool, 
            config,
            authenticated: Mutex::new(false),
        }
    }

    async fn verify_token(&self, token: &str) -> bool {
        let decoding_key = DecodingKey::from_secret(self.config.get_api_key().as_bytes());
        let mut validation = Validation::new(Algorithm::HS512);
        validation.validate_exp = true;
        
        match decode::<Claims>(token, &decoding_key, &validation) {
            Ok(_) => true,
            Err(e) => {
                warn!("P2P Authentication failed: {}", e);
                false
            }
        }
    }
}

#[async_trait]
impl P2PHandler for IngestHandler {
    async fn handle_message(&self, msg: Message) -> Np2pResult<Option<Message>> {
        match msg {
            Message::Authenticate { token } => {
                let success = self.verify_token(&token).await;
                let mut auth = self.authenticated.lock().await;
                *auth = success;
                
                return Ok(Some(Message::AuthenticateResponse {
                    success,
                    message: if success { "Authenticated".into() } else { "Invalid token".into() },
                }));
            }

            Message::LoginRequest { username, password_hash: password } => {
                info!("P2P Login request for user: {}", username);
                
                let client = match self.pool.get().await {
                    Ok(client) => client,
                    Err(e) => {
                        error!("Database connection error in P2P Login: {}", e);
                        return Ok(Some(Message::LoginResponse {
                            success: false,
                            token: None,
                            message: "Internal server error (DB)".into(),
                        }));
                    }
                };

                let query = "SELECT id, username, password_hash, role, is_active FROM users WHERE username = $1";
                let row = match client.query_opt(query, &[&username]).await {
                    Ok(Some(row)) => row,
                    Ok(None) => {
                        return Ok(Some(Message::LoginResponse {
                            success: false,
                            token: None,
                            message: "Invalid username or password".into(),
                        }));
                    }
                    Err(e) => {
                        error!("Database query error in P2P Login: {}", e);
                        return Ok(Some(Message::LoginResponse {
                            success: false,
                            token: None,
                            message: "Internal server error".into(),
                        }));
                    }
                };

                let user_id: Uuid = row.get("id");
                let db_password_hash: String = row.get("password_hash");
                let role: String = row.get("role");
                let is_active: bool = row.get("is_active");

                if !is_active {
                    return Ok(Some(Message::LoginResponse {
                        success: false,
                        token: None,
                        message: "Account is disabled".into(),
                    }));
                }

                match verify_password(&password, &db_password_hash) {
                    Ok(true) => {
                        let expiration_time = chrono::Utc::now() + chrono::Duration::days(7);
                        let claims = Claims {
                            user_id: user_id.to_string(),
                            username: username.clone(),
                            email: String::new(),
                            role: role.clone(),
                            exp: expiration_time.timestamp() as usize,
                        };

                        let token = encode(
                            &Header::new(Algorithm::HS512),
                            &claims,
                            &EncodingKey::from_secret(self.config.get_api_key().as_bytes())
                        );

                        match token {
                            Ok(t) => {
                                let mut auth = self.authenticated.lock().await;
                                *auth = true;
                                
                                Ok(Some(Message::LoginResponse {
                                    success: true,
                                    token: Some(t),
                                    message: "Login successful".into(),
                                }))
                            }
                            Err(e) => {
                                error!("Token generation error in P2P Login: {}", e);
                                Ok(Some(Message::LoginResponse {
                                    success: false,
                                    token: None,
                                    message: "Internal server error (Token)".into(),
                                }))
                            }
                        }
                    }
                    _ => {
                        Ok(Some(Message::LoginResponse {
                            success: false,
                            token: None,
                            message: "Invalid username or password".into(),
                        }))
                    }
                }
            }

            Message::UploadMediaRequest { device_id, file_hash, file_name, file_ext, data } => {
                // Check authentication
                {
                    let auth = self.authenticated.lock().await;
                    if !*auth {
                        return Ok(Some(Message::Error {
                            code: 401,
                            message: "Unauthorized. Please call Authenticate first.".into(),
                        }));
                    }
                }

                info!("📥 Received P2P upload request from {}: {} ({})", device_id, file_name, file_hash);
                
                let base_dir = if file_ext.to_lowercase() == "mp4" || file_ext.to_lowercase() == "mov" {
                    self.config.get_videos_dir()
                } else {
                    self.config.get_images_dir()
                };

                let dir_path = PathBuf::from(base_dir).join(&file_hash[0..2]);
                if !dir_path.exists() {
                    let _ = std::fs::create_dir_all(&dir_path);
                }
                
                let file_path = dir_path.join(format!("{}.{}", file_hash, file_ext));

                if let Err(e) = tokio::fs::write(&file_path, &data).await {
                    error!("Failed to write P2P upload to disk: {}", e);
                    return Ok(Some(Message::UploadMediaResponse {
                        success: false,
                        message: format!("Disk write error: {}", e),
                    }));
                }

                let table = if file_ext.to_lowercase() == "mp4" || file_ext.to_lowercase() == "mov" { "videos" } else { "images" };
                
                if let Ok(client) = self.pool.get().await {
                    let query = format!(
                        "INSERT INTO {} (deviceid, hash, name, ext, added_at) 
                         VALUES ($1, $2, $3, $4, NOW())
                         ON CONFLICT (deviceid, hash) DO NOTHING",
                        table
                    );
                    let _ = client.execute(&query, &[&device_id, &file_hash, &file_name, &file_ext]).await;
                }

                Ok(Some(Message::UploadMediaResponse {
                    success: true,
                    message: "File ingested successfully".to_string(),
                }))
            }
            _ => Ok(None),
        }
    }
}
