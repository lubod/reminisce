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

/// Authenticates a request by checking for a valid JWT in the Authorization header or
/// `token` query parameter. Returns the decoded claims on success.
pub fn authenticate_request(
    req: &HttpRequest,
    handler_name: &str,
    api_secret_env: &str,
) -> Result<Claims, HttpResponse> {
    if let Some(peer_addr) = req.peer_addr() {
        info!("{} request from: {}", handler_name, peer_addr);
    }

    let mut token = None;

    // 1. Try Authorization header
    if let Some(auth_header) = req.headers().get("Authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            if auth_str.starts_with("Bearer ") {
                token = Some(auth_str.trim_start_matches("Bearer ").to_string());
            }
        }
    }

    // 2. Try 'token' query parameter (useful for <img> tags)
    if token.is_none() {
        if let Ok(query) =
            web::Query::<std::collections::HashMap<String, String>>::from_query(req.query_string())
        {
            if let Some(t) = query.get("token") {
                token = Some(t.clone());
            }
        }
    }

    if let Some(token_str) = token {
        let validation = Validation::new(Algorithm::HS512);
        match jsonwebtoken::decode::<Claims>(
            &token_str,
            &DecodingKey::from_secret(api_secret_env.as_ref()),
            &validation,
        ) {
            Ok(token_data) => {
                log::debug!("JWT token validated successfully for {}.", handler_name);
                return Ok(token_data.claims);
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
}
