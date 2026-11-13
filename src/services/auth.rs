use actix_web::{ get, post, web, HttpResponse, HttpRequest };
use jsonwebtoken::{ encode, Algorithm, EncodingKey, Header };
use log::{ info, warn };
use serde::{ Deserialize, Serialize };
use sha2::{ Digest, Sha256 };
use utoipa::{ ToSchema };
use uuid::Uuid;

use crate::config::Config;
use crate::db::MainDbPool;
use crate::auth_utils::{ hash_password, verify_password };
use crate::metrics::{USER_REGISTRATIONS_TOTAL, USER_LOGINS_TOTAL, USER_LOGIN_FAILURES_TOTAL};
use crate::db_instrumentation::{instrumented_query_opt, instrumented_execute};

const REFRESH_TOKEN_TTL_DAYS: i32 = 90;

fn hash_refresh_token(raw: &str) -> String {
    hex::encode(Sha256::digest(raw.as_bytes()))
}

fn generate_refresh_token() -> (String, String) {
    let mut raw = [0u8; 32];
    rand::fill(&mut raw);
    let token = hex::encode(raw);
    let digest = hash_refresh_token(&token);
    (token, digest)
}

fn is_secure_cookie(config: &Config) -> bool {
    config.environment.as_deref() != Some("development")
        && config.environment.as_deref() != Some("dev")
}

fn build_session_cookie(
    name: &'static str,
    value: String,
    path: &'static str,
    max_age: actix_web::cookie::time::Duration,
    is_secure: bool,
) -> actix_web::cookie::Cookie<'static> {
    actix_web::cookie::Cookie::build(name, value)
        .path(path)
        .http_only(true)
        .same_site(actix_web::cookie::SameSite::Lax)
        .secure(is_secure)
        .max_age(max_age)
        .finish()
}

struct IssuedTokens {
    access_token: String,
    image_token: String,
    access_cookie: actix_web::cookie::Cookie<'static>,
    refresh_cookie: actix_web::cookie::Cookie<'static>,
}

fn build_jwt_tokens(
    config: &Config,
    user_id: Uuid,
    username: &str,
    role: &str,
    refresh_token: &str,
) -> Result<IssuedTokens, HttpResponse> {
    let api_key = config.get_api_key().map_err(|e| {
        HttpResponse::InternalServerError().json(serde_json::json!({
            "status": "error",
            "message": format!("Configuration error: {}", e)
        }))
    })?;

    let enc_key = EncodingKey::from_secret(api_key.as_bytes());
    let header = Header::new(Algorithm::HS512);

    let access_exp = (chrono::Utc::now() + chrono::Duration::days(7)).timestamp() as usize;
    let claims = Claims {
        user_id: user_id.to_string(),
        username: username.to_string(),
        email: String::new(),
        role: role.to_string(),
        exp: access_exp,
        scope: None,
    };
    let access_token = encode(&header, &claims, &enc_key).map_err(|e| {
        warn!("Failed to generate token: {:?}", e);
        HttpResponse::InternalServerError().json(serde_json::json!({
            "status": "error",
            "message": "Failed to generate token"
        }))
    })?;

    let image_exp = (chrono::Utc::now() + chrono::Duration::hours(24)).timestamp() as usize;
    let image_claims = Claims {
        user_id: user_id.to_string(),
        username: username.to_string(),
        email: String::new(),
        role: role.to_string(),
        exp: image_exp,
        scope: Some("media_read".to_string()),
    };
    let image_token = encode(&header, &image_claims, &enc_key).unwrap_or_default();

    let secure = is_secure_cookie(config);
    let access_cookie = build_session_cookie(
        "access_token",
        access_token.clone(),
        "/",
        actix_web::cookie::time::Duration::days(7),
        secure,
    );
    let refresh_cookie = build_session_cookie(
        "refresh_token",
        refresh_token.to_string(),
        "/api/auth",
        actix_web::cookie::time::Duration::days(REFRESH_TOKEN_TTL_DAYS as i64),
        secure,
    );

    Ok(IssuedTokens {
        access_token,
        image_token,
        access_cookie,
        refresh_cookie,
    })
}

fn build_auth_json_response(
    status: actix_web::http::StatusCode,
    issued: IssuedTokens,
    refresh_token: String,
    user_id: Uuid,
    username: String,
    role: String,
    include_status_ok: bool,
) -> HttpResponse {
    let mut body = serde_json::json!({
        "access_token": issued.access_token,
        "refresh_token": refresh_token,
        "image_token": issued.image_token,
        "expires_in_seconds": 7 * 24 * 60 * 60,
        "user": {
            "id": user_id.to_string(),
            "username": username,
            "role": role,
        }
    });
    if include_status_ok {
        body["status"] = serde_json::json!("ok");
    }
    let mut response = HttpResponse::build(status);
    response.cookie(issued.access_cookie);
    response.cookie(issued.refresh_cookie);
    response.json(body)
}

async fn store_refresh_token(
    client: &(impl deadpool_postgres::GenericClient + Sync),
    user_id: Uuid,
    token_hash: &str,
    device_id: Option<&str>,
) -> Result<(), HttpResponse> {
    client
        .execute(
            "INSERT INTO refresh_tokens (user_id, token_hash, device_id, expires_at)
             VALUES ($1, $2, $3, NOW() + make_interval(days => $4))",
            &[&user_id, &token_hash, &device_id, &REFRESH_TOKEN_TTL_DAYS],
        )
        .await
        .map_err(|e| {
            log::error!("Failed to store refresh token: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "status": "error",
                "message": "Failed to create session"
            }))
        })?;

    let _ = client
        .execute(
            "DELETE FROM refresh_tokens
             WHERE user_id = $1
               AND (expires_at < NOW() OR (revoked_at IS NOT NULL AND revoked_at < NOW() - INTERVAL '7 days'))",
            &[&user_id],
        )
        .await;

    Ok(())
}

// Claims structure with user information
#[derive(Serialize, Deserialize, ToSchema, Clone)]
#[schema(example = json!({
    "user_id": "550e8400-e29b-41d4-a716-446655440000",
    "username": "john_doe",
    "role": "user",
    "exp": 16725225600i64
}))]
pub struct Claims {
    pub user_id: String,   // UUID
    pub username: String,
    #[serde(default)]
    pub email: String,
    pub role: String,      // admin/user/viewer
    pub exp: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>, // optional scope restriction (e.g. "media_read")
}

use actix_web::{FromRequest, dev::Payload, Error as ActixError};
use futures_util::future::{ready, LocalBoxFuture};

impl FromRequest for Claims {
    type Error = ActixError;
    type Future = LocalBoxFuture<'static, Result<Self, Self::Error>>;

    fn from_request(req: &actix_web::HttpRequest, _payload: &mut Payload) -> Self::Future {
        // Get the API secret from Config app data
        let secret = if let Some(config) = req.app_data::<web::Data<Config>>() {
            config.api_secret_key.clone().unwrap_or_default()
        } else {
            return Box::pin(ready(Err(actix_web::error::ErrorInternalServerError("Config not available"))));
        };

        // Extract token from:
        // 1. Authorization header
        let mut token = None;
        if let Some(auth_header) = req.headers().get("Authorization") {
            if let Ok(auth_str) = auth_header.to_str() {
                if auth_str.starts_with("Bearer ") {
                    token = Some(auth_str.trim_start_matches("Bearer ").to_string());
                }
            }
        }

        // 2. Cookie 'access_token'
        if token.is_none() {
            if let Some(cookie) = req.cookie("access_token") {
                token = Some(cookie.value().to_string());
            }
        }

        // NOTE: tokens in query strings are deliberately not accepted — they leak
        // into access logs, browser history and Referer headers.

        let pool = req.app_data::<web::Data<MainDbPool>>().cloned();
        let path = req.path().to_string();
        let method = req.method().clone();

        let fut = async move {
            let token_str = match token {
                Some(t) => t,
                None => return Err(actix_web::error::ErrorUnauthorized("Authentication required")),
            };

            let validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS512);
            let token_data = jsonwebtoken::decode::<Claims>(
                &token_str,
                &jsonwebtoken::DecodingKey::from_secret(secret.as_ref()),
                &validation,
            ).map_err(|_| actix_web::error::ErrorUnauthorized("Invalid token"))?;

            let claims = token_data.claims;

            // Check scope restriction if present
            if let Some(ref scope) = claims.scope {
                if scope == "media_read" {
                    let is_get = method == actix_web::http::Method::GET;
                    let is_media_path = path.starts_with("/api/image/")
                        || path.starts_with("/api/video/")
                        || path.starts_with("/api/face/")
                        || path.starts_with("/api/faces/")
                        || path.starts_with("/api/thumbnail/");
                    if !is_get || !is_media_path {
                        return Err(actix_web::error::ErrorForbidden("Token is restricted to media read access only"));
                    }
                }
            }

            let user_uuid = uuid::Uuid::parse_str(&claims.user_id)
                .map_err(|_| actix_web::error::ErrorUnauthorized("Invalid user ID in token"))?;

            let (user_role, user_active) = if let Some(pool) = pool {
                crate::auth_utils::get_cached_or_query_user_status(&user_uuid, pool.as_ref()).await?
            } else {
                log::error!("MainDbPool app data is missing in Claims FromRequest");
                return Err(actix_web::error::ErrorInternalServerError("Database configuration error"));
            };

            if !user_active {
                return Err(actix_web::error::ErrorUnauthorized("Account is disabled"));
            }
            let mut claims_updated = claims;
            claims_updated.role = user_role;
            Ok(claims_updated)
        };

        Box::pin(fut)
    }
}

// User login request
#[derive(Serialize, Deserialize, ToSchema)]
#[schema(example = json!({
    "username": "john_doe",
    "password": "secure_password_123"
}))]
pub struct UserLoginRequest {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub device_id: Option<String>,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct RefreshTokenRequest {
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub device_id: Option<String>,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct LogoutRequest {
    #[serde(default)]
    pub refresh_token: Option<String>,
}

// --- Setup endpoints (first-run only) ---

#[derive(Serialize, Deserialize, ToSchema)]
pub struct SetupRequest {
    pub username: String,
    pub password: String,
}

/// Returns whether the server needs initial setup (no users exist yet).
#[utoipa::path(
    get,
    path = "/api/auth/setup-status",
    responses(
        (status = 200, description = "Returns if setup is needed", body = serde_json::Value),
        (status = 500, description = "Server error")
    ),
    tag = "Authentication"
)]
#[get("/auth/setup-status")]
pub async fn setup_status(pool: web::Data<MainDbPool>) -> HttpResponse {
    let client = match pool.0.get().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError().finish(),
    };
    match client.query_one("SELECT COUNT(*) FROM users", &[]).await {
        Ok(row) => {
            let count: i64 = row.get(0);
            HttpResponse::Ok().json(serde_json::json!({ "needs_setup": count == 0 }))
        }
        Err(_) => HttpResponse::InternalServerError().finish(),
    }
}

/// Creates the first admin account. Returns 403 if any user already exists.
#[utoipa::path(
    post,
    path = "/api/auth/setup",
    request_body = SetupRequest,
    responses(
        (status = 201, description = "Admin user created successfully", body = serde_json::Value),
        (status = 400, description = "Invalid request format"),
        (status = 403, description = "Setup is already completed"),
        (status = 500, description = "Server error")
    ),
    tag = "Authentication"
)]
#[post("/auth/setup")]
pub async fn setup_admin(
    req_body: web::Json<SetupRequest>,
    pool: web::Data<MainDbPool>,
    config: Option<web::Data<Config>>,
) -> HttpResponse {
    let username = req_body.username.trim();
    if username.len() < 3 || req_body.password.len() < 8 {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "status": "error",
            "message": "Username must be ≥3 chars and password ≥8 chars"
        }));
    }

    let mut client = match pool.0.get().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError().finish(),
    };

    // Serialize concurrent first-run setup requests with a transaction-scoped advisory
    // lock so two simultaneous requests can't both see COUNT(*)==0 and create admins.
    let tx = match client.transaction().await {
        Ok(t) => t,
        Err(e) => {
            log::error!("Failed to begin setup transaction: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "status": "error",
                "message": "Database transaction failed"
            }));
        }
    };
    if let Err(e) = tx.query_opt(
        "SELECT pg_advisory_xact_lock($1)",
        &[&0x524D_4E53_i64], // arbitrary app-wide constant for "setup"
    ).await {
        log::error!("Failed to acquire setup lock: {}", e);
        return HttpResponse::InternalServerError().json(serde_json::json!({
            "status": "error",
            "message": "Failed to acquire setup lock"
        }));
    }

    let row = match tx.query_one("SELECT COUNT(*) FROM users", &[]).await {
        Ok(r) => r,
        Err(e) => {
            log::error!("Failed to query user count during setup: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "status": "error",
                "message": "Database query failed"
            }));
        }
    };
    let count: i64 = row.get(0);
    if count > 0 {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "status": "error",
            "message": "Setup already completed"
        }));
    }

    let password_hash = match hash_password(&req_body.password) {
        Ok(h) => h,
        Err(_) => return HttpResponse::InternalServerError().finish(),
    };

    let email = format!("{}@local", username);
    match tx.query_one(
        "INSERT INTO users (username, email, password_hash, role) VALUES ($1, $2, $3, 'admin') RETURNING id",
        &[&username, &email, &password_hash],
    ).await {
        Ok(inserted) => {
            let user_id: Uuid = inserted.get(0);
            if let Some(cfg) = config {
                let (refresh_token, token_hash) = generate_refresh_token();
                if let Err(resp) = store_refresh_token(&tx, user_id, &token_hash, None).await {
                    return resp;
                }
                let issued = match build_jwt_tokens(&cfg, user_id, username, "admin", &refresh_token) {
                    Ok(t) => t,
                    Err(resp) => return resp,
                };
                if tx.commit().await.is_err() {
                    log::error!("Failed to commit setup transaction");
                    return HttpResponse::InternalServerError().json(serde_json::json!({
                        "status": "error",
                        "message": "Failed to commit setup"
                    }));
                }
                info!("Initial admin account created: {}", username);
                USER_REGISTRATIONS_TOTAL.inc();
                return build_auth_json_response(
                    actix_web::http::StatusCode::CREATED,
                    issued,
                    refresh_token,
                    user_id,
                    username.to_string(),
                    "admin".to_string(),
                    true,
                );
            }

            if tx.commit().await.is_err() {
                log::error!("Failed to commit setup transaction");
                return HttpResponse::InternalServerError().json(serde_json::json!({
                    "status": "error",
                    "message": "Failed to commit setup"
                }));
            }
            info!("Initial admin account created: {}", username);
            USER_REGISTRATIONS_TOTAL.inc();
            HttpResponse::Created().json(serde_json::json!({ "status": "ok" }))
        }
        Err(e) => {
            warn!("Setup failed: {:?}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "status": "error", "message": "Setup failed"
            }))
        }
    }
}

// User login endpoint (with username/password)
#[utoipa::path(
    post,
    path = "/auth/user-login",
    request_body = UserLoginRequest,
    responses(
        (status = 200, description = "Login successful", body = serde_json::Value),
        (status = 401, description = "Invalid credentials"),
        (status = 500, description = "Server error")
    )
)]
#[post("/auth/user-login")]
pub async fn user_login(
    req_body: web::Json<UserLoginRequest>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
) -> HttpResponse {
    info!("User login attempt for username: {}", req_body.username);

    match perform_login(
        &pool,
        &config,
        &req_body.username,
        &req_body.password,
        req_body.device_id.as_deref(),
    )
    .await
    {
        Ok(outcome) => build_auth_json_response(
            actix_web::http::StatusCode::OK,
            outcome.issued,
            outcome.refresh_token,
            outcome.user_id,
            outcome.username,
            outcome.role,
            false,
        ),
        Err(resp) => resp,
    }
}

/// Native form login used by the browser login form. Performs the same auth as
/// `user_login`, sets the same httpOnly session cookie, then redirects to the app
/// root. A real form POST followed by a navigation is exactly what makes the
/// browser's password manager offer to save credentials.
#[utoipa::path(
    post,
    path = "/auth/user-login-form",
    request_body = UserLoginRequest,
    responses(
        (status = 303, description = "Login successful — redirect to app root"),
        (status = 401, description = "Invalid credentials")
    )
)]
#[post("/auth/user-login-form")]
pub async fn user_login_form(
    req_body: web::Form<UserLoginRequest>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
) -> HttpResponse {
    info!("Form login attempt for username: {}", req_body.username);

    match perform_login(
        &pool,
        &config,
        &req_body.username,
        &req_body.password,
        req_body.device_id.as_deref(),
    )
    .await
    {
        Ok(outcome) => {
            let mut response = HttpResponse::Found();
            response.insert_header((actix_web::http::header::LOCATION, "/"));
            response.cookie(outcome.issued.access_cookie);
            response.cookie(outcome.issued.refresh_cookie);
            response.finish()
        }
        Err(_) => {
            let mut response = HttpResponse::Found();
            response.insert_header((actix_web::http::header::LOCATION, "/login?error=1"));
            response.finish()
        }
    }
}

/// Shared login logic: validates credentials and, on success, issues a 7-day
/// httpOnly session cookie plus a 90-day rotating refresh token. Used by both
/// the JSON API (`user_login`) and the native form login (`user_login_form`).
struct LoginOutcome {
    issued: IssuedTokens,
    refresh_token: String,
    user_id: Uuid,
    username: String,
    role: String,
}

async fn perform_login(
    pool: &web::Data<MainDbPool>,
    config: &web::Data<Config>,
    username: &str,
    password: &str,
    device_id: Option<&str>,
) -> Result<LoginOutcome, HttpResponse> {
    if username.trim().is_empty() || password.is_empty() {
        return Err(HttpResponse::BadRequest().json(serde_json::json!({
            "status": "error",
            "message": "Username and password are required"
        })));
    }

    // Per-account brute-force lockout (in addition to the per-IP bucket):
    // repeated failures for one username within the window fail closed.
    if !crate::rate_limit::login_allowed_for_account(username) {
        warn!("Login blocked by account lockout: {}", username);
        USER_LOGIN_FAILURES_TOTAL.inc();
        return Err(HttpResponse::TooManyRequests().json(serde_json::json!({
            "status": "error",
            "message": "Too many failed attempts. Try again later."
        })));
    }

    let client = match pool.0.get().await {
        Ok(client) => client,
        Err(e) => {
            warn!("Failed to get database connection: {:?}", e);
            return Err(HttpResponse::InternalServerError().json(serde_json::json!({
                "status": "error",
                "message": "Database connection failed"
            })));
        }
    };

    let query = "SELECT id, username, password_hash, role, is_active FROM users WHERE username = $1";
    let row = match instrumented_query_opt(&client, query, &[&username], "user_login_query").await {
        Ok(Some(row)) => row,
        Ok(None) => {
            warn!("User not found: {}", username);
            USER_LOGIN_FAILURES_TOTAL.inc();
            crate::rate_limit::record_login_failure(username);
            return Err(HttpResponse::Unauthorized().json(serde_json::json!({
                "status": "error",
                "message": "Invalid username or password"
            })));
        }
        Err(e) => {
            warn!("Database error during login: {:?}", e);
            return Err(HttpResponse::InternalServerError().json(serde_json::json!({
                "status": "error",
                "message": "Login failed"
            })));
        }
    };

    let user_id: Uuid = row.get("id");
    let db_username: String = row.get("username");
    let password_hash: String = row.get("password_hash");
    let role: String = row.get("role");
    let is_active: bool = row.get("is_active");

    if !is_active {
        warn!("Inactive user attempted login: {}", username);
        USER_LOGIN_FAILURES_TOTAL.inc();
        crate::rate_limit::record_login_failure(username);
        return Err(HttpResponse::Unauthorized().json(serde_json::json!({
            "status": "error",
            "message": "Account is disabled"
        })));
    }

    match verify_password(password, &password_hash) {
        Ok(true) => {
            let (refresh_token, token_hash) = generate_refresh_token();
            let issued = build_jwt_tokens(config, user_id, &db_username, &role, &refresh_token)?;
            store_refresh_token(&client, user_id, &token_hash, device_id).await?;

            let _ = instrumented_execute(
                &client,
                "UPDATE users SET last_login_at = NOW() WHERE id = $1",
                &[&user_id],
                "update_last_login",
            )
            .await;

            info!("User logged in successfully: {}", db_username);
            USER_LOGINS_TOTAL.inc();
            crate::rate_limit::clear_login_failures(username);

            Ok(LoginOutcome {
                issued,
                refresh_token,
                user_id,
                username: db_username,
                role,
            })
        }
        Ok(false) => {
            warn!("Invalid password for user: {}", username);
            USER_LOGIN_FAILURES_TOTAL.inc();
            crate::rate_limit::record_login_failure(username);
            Err(HttpResponse::Unauthorized().json(serde_json::json!({
                "status": "error",
                "message": "Invalid username or password"
            })))
        }
        Err(e) => {
            warn!("Password verification error: {:?}", e);
            Err(HttpResponse::InternalServerError().json(serde_json::json!({
                "status": "error",
                "message": "Authentication failed"
            })))
        }
    }
}

fn extract_refresh_token(req: &HttpRequest, body_token: Option<&str>) -> Option<String> {
    body_token
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            req.cookie("refresh_token")
                .map(|c| c.value().trim().to_string())
                .filter(|s| !s.is_empty())
        })
}

/// Exchange a valid rotating refresh token (from JSON body or `refresh_token` HttpOnly cookie)
/// for a fresh access token, image token, and replacement refresh token.
#[utoipa::path(
    post,
    path = "/api/auth/refresh",
    request_body(content = Option<RefreshTokenRequest>, content_type = "application/json"),
    responses(
        (status = 200, description = "Token refreshed", body = serde_json::Value),
        (status = 401, description = "Invalid, expired, or revoked refresh token")
    ),
    tag = "Authentication"
)]
#[post("/auth/refresh")]
pub async fn user_refresh(
    req: HttpRequest,
    req_body: Option<web::Json<RefreshTokenRequest>>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
) -> HttpResponse {
    let raw_token = match extract_refresh_token(
        &req,
        req_body.as_ref().and_then(|b| b.refresh_token.as_deref()),
    ) {
        Some(t) => t,
        None => {
            return HttpResponse::Unauthorized().json(serde_json::json!({
                "status": "error",
                "message": "Refresh token required"
            }));
        }
    };
    let token_hash = hash_refresh_token(&raw_token);

    let mut client = match pool.0.get().await {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to get database connection for token refresh: {:?}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "status": "error",
                "message": "Database connection failed"
            }));
        }
    };

    let tx = match client.transaction().await {
        Ok(t) => t,
        Err(e) => {
            log::error!("Failed to start refresh transaction: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "status": "error",
                "message": "Token refresh failed"
            }));
        }
    };

    let row = match tx
        .query_opt(
            "UPDATE refresh_tokens rt
             SET revoked_at = NOW()
             FROM users u
             WHERE rt.token_hash = $1
               AND rt.revoked_at IS NULL
               AND rt.expires_at > NOW()
               AND u.id = rt.user_id
               AND u.is_active = TRUE
             RETURNING rt.user_id, rt.device_id, u.username, u.role",
            &[&token_hash],
        )
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            return HttpResponse::Unauthorized().json(serde_json::json!({
                "status": "error",
                "message": "Invalid or expired refresh token"
            }));
        }
        Err(e) => {
            log::error!("Failed to validate refresh token: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "status": "error",
                "message": "Token refresh failed"
            }));
        }
    };

    let user_id: Uuid = row.get("user_id");
    let existing_device_id: Option<String> = row.get("device_id");
    let username: String = row.get("username");
    let role: String = row.get("role");
    let effective_device_id = req_body
        .as_ref()
        .and_then(|b| b.device_id.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or(existing_device_id.as_deref());

    let (new_refresh, new_hash) = generate_refresh_token();
    let issued = match build_jwt_tokens(&config, user_id, &username, &role, &new_refresh) {
        Ok(t) => t,
        Err(resp) => return resp,
    };

    if let Err(resp) = store_refresh_token(&tx, user_id, &new_hash, effective_device_id).await {
        return resp;
    }

    if let Err(e) = tx.commit().await {
        log::error!("Failed to commit refresh token rotation: {}", e);
        return HttpResponse::InternalServerError().json(serde_json::json!({
            "status": "error",
            "message": "Token refresh failed"
        }));
    }

    build_auth_json_response(
        actix_web::http::StatusCode::OK,
        issued,
        new_refresh,
        user_id,
        username,
        role,
        false,
    )
}

// User logout endpoint (revokes refresh token and clears HttpOnly cookies)
#[utoipa::path(
    post,
    path = "/api/auth/logout",
    request_body(content = Option<LogoutRequest>, content_type = "application/json"),
    responses(
        (status = 200, description = "Logged out successfully", body = serde_json::Value)
    ),
    tag = "Authentication"
)]
#[post("/auth/logout")]
pub async fn user_logout(
    req: HttpRequest,
    pool: Option<web::Data<MainDbPool>>,
    req_body: Option<web::Json<LogoutRequest>>,
) -> HttpResponse {
    if let (Some(raw), Some(p)) = (
        extract_refresh_token(&req, req_body.as_ref().and_then(|b| b.refresh_token.as_deref())),
        pool,
    ) {
        let token_hash = hash_refresh_token(&raw);
        if let Ok(client) = p.0.get().await {
            let _ = client
                .execute(
                    "UPDATE refresh_tokens SET revoked_at = NOW() WHERE token_hash = $1 AND revoked_at IS NULL",
                    &[&token_hash],
                )
                .await;
        }
    }

    let secure = req
        .app_data::<web::Data<Config>>()
        .map(|c| is_secure_cookie(c))
        .unwrap_or(false);
    let access_cookie = build_session_cookie(
        "access_token",
        String::new(),
        "/",
        actix_web::cookie::time::Duration::ZERO,
        secure,
    );
    let refresh_cookie = build_session_cookie(
        "refresh_token",
        String::new(),
        "/api/auth",
        actix_web::cookie::time::Duration::ZERO,
        secure,
    );

    let mut response = HttpResponse::Ok();
    response.cookie(access_cookie);
    response.cookie(refresh_cookie);
    response.json(serde_json::json!({
        "status": "ok",
        "message": "Logged out successfully"
    }))
}

// User details endpoint (returns currently authenticated user session)
#[utoipa::path(
    get,
    path = "/api/auth/me",
    responses(
        (status = 200, description = "Current user information", body = serde_json::Value),
        (status = 401, description = "Unauthorized")
    ),
    tag = "Authentication"
)]
#[get("/auth/me")]
pub async fn get_me(req: HttpRequest, claims: Claims) -> HttpResponse {
    let mut token = String::new();
    if let Some(auth_header) = req.headers().get("Authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            if auth_str.starts_with("Bearer ") {
                token = auth_str.trim_start_matches("Bearer ").to_string();
            }
        }
    }
    if token.is_empty() {
        if let Some(cookie) = req.cookie("access_token") {
            token = cookie.value().to_string();
        }
    }

    let config = match req.app_data::<web::Data<Config>>() {
        Some(c) => c,
        None => return HttpResponse::InternalServerError().finish(),
    };

    let image_token_exp = chrono::Utc::now() + chrono::Duration::hours(24);
    let image_claims = Claims {
        user_id: claims.user_id.clone(),
        username: claims.username.clone(),
        email: claims.email.clone(),
        role: claims.role.clone(),
        exp: image_token_exp.timestamp() as usize,
        scope: Some("media_read".to_string()),
    };

    let api_key = match config.get_api_key() {
        Ok(k) => k,
        Err(e) => return HttpResponse::InternalServerError().json(serde_json::json!({
            "status": "error",
            "message": format!("Configuration error: {}", e)
        })),
    };

    let image_token = encode(
        &Header::new(Algorithm::HS512),
        &image_claims,
        &EncodingKey::from_secret(api_key.as_bytes())
    ).unwrap_or_default();

    HttpResponse::Ok().json(serde_json::json!({
        "id": claims.user_id,
        "username": claims.username,
        "role": claims.role,
        "access_token": token,
        "image_token": image_token
    }))
}