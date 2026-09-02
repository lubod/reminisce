use actix_web::{web, HttpRequest, HttpResponse};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use log::{error, info, warn};

use crate::services::auth::Claims;

/// Parse a user_id string into a UUID, returning 400 on failure.
pub fn parse_user_uuid(user_id: &str) -> Result<uuid::Uuid, actix_web::Error> {
    uuid::Uuid::parse_str(user_id).map_err(|e| {
        error!("Failed to parse user_id as UUID: {}", e);
        actix_web::error::ErrorBadRequest("Invalid user ID")
    })
}

/// Ensures a user from JWT claims exists in the local database.
/// Auto-provisions the user if they don't exist (relay is the source of truth for auth).
pub async fn ensure_user_exists(
    client: &tokio_postgres::Client,
    claims: &Claims,
) -> Result<(), actix_web::Error> {
    let user_uuid = uuid::Uuid::parse_str(&claims.user_id).map_err(|e| {
        error!("Failed to parse user_id as UUID: {}", e);
        actix_web::error::ErrorBadRequest("Invalid user ID")
    })?;

    let exists = client
        .query_opt("SELECT 1 FROM users WHERE id = $1", &[&user_uuid])
        .await
        .map_err(|e| {
            error!("Failed to check user existence: {}", e);
            actix_web::error::ErrorInternalServerError("Database error")
        })?
        .is_some();

    if !exists {
        let email = if claims.email.is_empty() {
            format!("{}@relay", claims.username)
        } else {
            claims.email.clone()
        };
        info!(
            "Auto-provisioning user from relay JWT: id={}, username={}, email={}, role={}",
            claims.user_id, claims.username, email, claims.role
        );
        client
            .execute(
                "INSERT INTO users (id, username, email, password_hash, role) \
                 VALUES ($1, $2, $3, $4, $5) ON CONFLICT (id) DO NOTHING",
                &[&user_uuid, &claims.username, &email, &"relay-managed", &claims.role],
            )
            .await
            .map_err(|e| {
                error!("Failed to auto-provision user: {}", e);
                actix_web::error::ErrorInternalServerError("Failed to create user")
            })?;
    }

    Ok(())
}

// Media-read-scoped tokens (the short-lived `image_token` handed out for `<img>` /
// media URLs) must never unlock anything that is not raw media byte-serving.
// Every other handler requires a full session token.
const MEDIA_READ_SAFE_HANDLERS: &[&str] = &["get_image", "get_video"];

/// Authenticates a request by checking for a valid JWT in the Authorization header or
/// `token` query parameter. Returns the decoded claims on success.

#[derive(Clone)]
pub struct CachedUserStatus {
    pub role: String,
    pub is_active: bool,
    pub fetched_at: std::time::Instant,
}

static USER_CACHE: std::sync::OnceLock<std::sync::Arc<std::sync::RwLock<std::collections::HashMap<uuid::Uuid, CachedUserStatus>>>> = std::sync::OnceLock::new();

pub fn user_cache() -> std::sync::Arc<std::sync::RwLock<std::collections::HashMap<uuid::Uuid, CachedUserStatus>>> {
    USER_CACHE.get_or_init(|| std::sync::Arc::new(std::sync::RwLock::new(std::collections::HashMap::new()))).clone()
}

pub fn invalidate_user_cache(user_id: &uuid::Uuid) {
    if let Some(cache) = USER_CACHE.get() {
        let mut guard = cache.write().unwrap_or_else(|e| e.into_inner());
        guard.remove(user_id);
    }
}

pub async fn get_cached_or_query_user_status(
    user_uuid: &uuid::Uuid,
    pool: &crate::db::MainDbPool,
) -> Result<(String, bool), actix_web::Error> {
    let cache = user_cache();
    let hit = {
        let guard = cache.read().unwrap_or_else(|e| e.into_inner());
        if guard.len() > 500 {
            drop(guard);
            let mut wguard = cache.write().unwrap_or_else(|e| e.into_inner());
            wguard.retain(|_, v| v.fetched_at.elapsed() < std::time::Duration::from_secs(60));
        }
        let guard = cache.read().unwrap_or_else(|e| e.into_inner());
        guard.get(user_uuid)
            .filter(|c| c.fetched_at.elapsed() < std::time::Duration::from_secs(5))
            .map(|c| (c.role.clone(), c.is_active))
    };

    if let Some((r, a)) = hit {
        return Ok((r, a));
    }

    let client = pool.0.get().await.map_err(|e| {
        log::error!("DB connection error in get_cached_or_query_user_status: {:?}", e);
        actix_web::error::ErrorInternalServerError("Database connection failed")
    })?;

    let row = client.query_opt(
        "SELECT role, is_active FROM users WHERE id = $1",
        &[user_uuid]
    ).await.map_err(|e| {
        log::error!("DB query error in get_cached_or_query_user_status: {:?}", e);
        actix_web::error::ErrorInternalServerError("Database error")
    })?;

    if let Some(row) = row {
        let r: String = row.get("role");
        let a: bool = row.get("is_active");
        let mut cache_write = cache.write().unwrap_or_else(|e| e.into_inner());
        cache_write.insert(*user_uuid, CachedUserStatus {
            role: r.clone(),
            is_active: a,
            fetched_at: std::time::Instant::now(),
        });
        Ok((r, a))
    } else {
        Err(actix_web::error::ErrorUnauthorized("User not found"))
    }
}

pub async fn authenticate_request(
    req: &HttpRequest,
    handler_name: &str,
    api_secret_env: Result<&str, &'static str>,
) -> Result<Claims, HttpResponse> {
    if let Some(peer_addr) = req.peer_addr() {
        info!("{} request from: {}", handler_name, peer_addr);
    }

    let api_secret = match api_secret_env {
        Ok(s) => s,
        Err(e) => {
            log::error!("Authentication failed: {}", e);
            return Err(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": e
            })));
        }
    };

    let mut token = None;

    // 1. Try Authorization header
    if let Some(auth_header) = req.headers().get("Authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            if auth_str.starts_with("Bearer ") {
                token = Some(auth_str.trim_start_matches("Bearer ").to_string());
            }
        }
    }

    // 2. Try 'access_token' HTTP cookie
    if token.is_none() {
        if let Some(cookie) = req.cookie("access_token") {
            token = Some(cookie.value().to_string());
        }
    }

    if let Some(token_str) = token {
        let validation = Validation::new(Algorithm::HS512);
        match jsonwebtoken::decode::<Claims>(
            &token_str,
            &DecodingKey::from_secret(api_secret.as_bytes()),
            &validation,
        ) {
            Ok(token_data) => {
                log::debug!("JWT token validated successfully for {}.", handler_name);
                let claims = token_data.claims;

                // Enforce token scope: a media_read-scoped token is downgraded to raw
                // media access only; it cannot drive mutations or admin/management
                // endpoints (previously the image_token worked as full privilege here).
                if let Some(scope) = claims.scope.as_deref() {
                    if scope == "media_read" && !MEDIA_READ_SAFE_HANDLERS.contains(&handler_name) {
                        log::warn!("media_read-scoped token rejected for handler '{}'", handler_name);
                        return Err(HttpResponse::Forbidden().json(serde_json::json!({
                            "error": "Insufficient token scope"
                        })));
                    }
                }

                let user_uuid = match uuid::Uuid::parse_str(&claims.user_id) {
                    Ok(u) => u,
                    Err(_) => return Err(HttpResponse::Unauthorized().json(serde_json::json!({"error": "Invalid user ID in token"}))),
                };

                if let Some(pool) = req.app_data::<web::Data<crate::db::MainDbPool>>() {
                    let (role, is_active) = match get_cached_or_query_user_status(&user_uuid, pool.as_ref()).await {
                        Ok(status) => status,
                        Err(_) => {
                            return Err(HttpResponse::Unauthorized().json(serde_json::json!({"error": "User not found or database error"})));
                        }
                    };

                    if !is_active {
                        return Err(HttpResponse::Unauthorized().json(serde_json::json!({"error": "Account is disabled"})));
                    }
                    let mut claims_updated = claims;
                    claims_updated.role = role;
                    return Ok(claims_updated);
                } else {
                    log::error!("MainDbPool app data is missing in authenticate_request");
                    return Err(HttpResponse::InternalServerError().json(serde_json::json!({"error": "Database configuration error"})));
                }
            }
            Err(e) => {
                warn!("JWT validation failed for {}: {:?}", handler_name, e);
            }
        }
    }

    warn!(
        "Authentication failed for {}: No valid JWT token found.",
        handler_name
    );
    Err(HttpResponse::Unauthorized().json("Authentication required"))
}

// ---- Password hashing (Argon2) ----

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};

/// Hash a password using Argon2id
pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(password.as_bytes(), &salt)?
        .to_string();
    Ok(password_hash)
}

/// Verify a password against a hash
pub fn verify_password(password: &str, hash: &str) -> Result<bool, argon2::password_hash::Error> {
    let parsed_hash = PasswordHash::new(hash)?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_and_verify_password() {
        let password = "test_password_123";
        let hash = hash_password(password).expect("Failed to hash password");

        assert!(verify_password(password, &hash).expect("Failed to verify password"));


        assert!(!verify_password("wrong_password", &hash).expect("Failed to verify password"));
    }

    #[test]
    fn test_parse_user_uuid() {
        let valid = "550e8400-e29b-41d4-a716-446655440000";
        assert!(parse_user_uuid(valid).is_ok());
        assert_eq!(parse_user_uuid(valid).unwrap().to_string(), valid);

        assert!(parse_user_uuid("not-a-uuid").is_err());
        assert!(parse_user_uuid("").is_err());
        assert!(parse_user_uuid(&"a".repeat(40)).is_err());
    }

    #[test]
    fn test_user_cache_insert_lookup_and_invalidate() {
        let user_id = uuid::Uuid::new_v4();
        let cache = user_cache();
        {
            let mut guard = cache.write().unwrap();
            guard.insert(user_id, CachedUserStatus {
                role: "admin".to_string(),
                is_active: true,
                fetched_at: std::time::Instant::now(),
            });
        }
        {
            let guard = cache.read().unwrap();
            let entry = guard.get(&user_id).unwrap();
            assert_eq!(entry.role, "admin");
            assert!(entry.is_active);
        }
        invalidate_user_cache(&user_id);
        {
            let guard = cache.read().unwrap();
            assert!(guard.get(&user_id).is_none());
        }
    }
}