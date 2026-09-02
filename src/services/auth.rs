use actix_web::{ get, post, web, HttpResponse, HttpRequest };
use jsonwebtoken::{ encode, Algorithm, EncodingKey, Header };
use log::{ info, warn };
use serde::{ Deserialize, Serialize };
use utoipa::{ ToSchema };
use validator::Validate;
use uuid::Uuid;

use crate::config::Config;
use crate::db::MainDbPool;
use crate::auth_utils::{ hash_password, verify_password };
use crate::metrics::{USER_REGISTRATIONS_TOTAL, USER_LOGINS_TOTAL, USER_LOGIN_FAILURES_TOTAL};
use crate::db_instrumentation::{instrumented_query_opt, instrumented_execute};

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
                        || path.starts_with("/api/media/")
                        || path.starts_with("/api/images/")
                        || path.starts_with("/api/videos/")
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

// User registration request
#[derive(Serialize, Deserialize, Validate, ToSchema)]
#[schema(example = json!({
    "username": "john_doe",
    "email": "john@example.com",
    "password": "secure_password_123"
}))]
pub struct RegisterRequest {
    #[validate(length(min = 3, max = 255))]
    pub username: String,
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 8))]
    pub password: String,
}

// User login request (new version with username/password)
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
}

// Public registration is disabled — users are created by admins only.
#[utoipa::path(
    post,
    path = "/api/auth/register",
    responses(
        (status = 403, description = "Forbidden - Public registration is disabled"),
    ),
    tag = "Authentication"
)]
#[post("/auth/register")]
pub async fn register_user() -> HttpResponse {
    HttpResponse::Forbidden().json(serde_json::json!({
        "status": "error",
        "message": "Registration is disabled. Contact your administrator."
    }))
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
) -> HttpResponse {
    if req_body.username.len() < 3 || req_body.password.len() < 8 {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "status": "error",
            "message": "Username must be ≥3 chars and password ≥8 chars"
        }));
    }

    let client = match pool.0.get().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError().finish(),
    };

    // Serialize concurrent first-run setup requests with a transaction-scoped advisory
    // lock so two simultaneous requests can't both see COUNT(*)==0 and create admins.
    let mut client = client;
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

    let email = format!("{}@local", req_body.username);
    match tx.execute(
        "INSERT INTO users (username, email, password_hash, role) VALUES ($1, $2, $3, 'admin')",
        &[&req_body.username, &email, &password_hash],
    ).await {
        Ok(_) => {
            if tx.commit().await.is_err() {
                log::error!("Failed to commit setup transaction");
                return HttpResponse::InternalServerError().json(serde_json::json!({
                    "status": "error",
                    "message": "Failed to commit setup"
                }));
            }
            info!("Initial admin account created: {}", req_body.username);
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

    match perform_login(&pool, &config, &req_body.username, &req_body.password).await {
        Ok(outcome) => {
            let image_token_exp = chrono::Utc::now() + chrono::Duration::hours(24);
            let image_claims = Claims {
                user_id: outcome.user_id.to_string(),
                username: outcome.username.clone(),
                email: String::new(),
                role: outcome.role.clone(),
                exp: image_token_exp.timestamp() as usize,
                scope: Some("media_read".to_string()),
            };
            let image_token = encode(
                &Header::new(Algorithm::HS512),
                &image_claims,
                &EncodingKey::from_secret(config.get_api_key().unwrap_or("").as_bytes()),
            ).unwrap_or_default();

            let mut response = HttpResponse::Ok();
            response.cookie(outcome.cookie);
            response.json(serde_json::json!({
                "access_token": outcome.access_token,
                "image_token": image_token,
                "user": {
                    "id": outcome.user_id.to_string(),
                    "username": outcome.username,
                    "role": outcome.role
                }
            }))
        }
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

    match perform_login(&pool, &config, &req_body.username, &req_body.password).await {
        Ok(outcome) => {
            let mut response = HttpResponse::Found();
            response.insert_header((actix_web::http::header::LOCATION, "/"));
            response.cookie(outcome.cookie);
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
/// httpOnly session cookie plus the access token. Used by both the JSON API
/// (`user_login`) and the native form login (`user_login_form`).
struct LoginOutcome {
    cookie: actix_web::cookie::Cookie<'static>,
    access_token: String,
    user_id: Uuid,
    username: String,
    role: String,
}

async fn perform_login(
    pool: &web::Data<MainDbPool>,
    config: &web::Data<Config>,
    username: &str,
    password: &str,
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
            let expiration_time = chrono::Utc::now() + chrono::Duration::days(7);
            let claims = Claims {
                user_id: user_id.to_string(),
                username: username.to_string(),
                email: String::new(),
                role: role.clone(),
                exp: expiration_time.timestamp() as usize,
                scope: None,
            };

            let api_key = match config.get_api_key() {
                Ok(k) => k,
                Err(e) => return Err(HttpResponse::InternalServerError().json(serde_json::json!({
                    "status": "error",
                    "message": format!("Configuration error: {}", e)
                }))),
            };

            let token = encode(
                &Header::new(Algorithm::HS512),
                &claims,
                &EncodingKey::from_secret(api_key.as_bytes()),
            );

            match token {
                Ok(t) => {
                    let _ = instrumented_execute(
                        &client,
                        "UPDATE users SET last_login_at = NOW() WHERE id = $1",
                        &[&user_id],
                        "update_last_login",
                    ).await;

                    info!("User logged in successfully: {}", username);
                    USER_LOGINS_TOTAL.inc();
                    crate::rate_limit::clear_login_failures(username);

                    let is_secure = config.environment.as_deref() != Some("development") && config.environment.as_deref() != Some("dev");
                    let cookie = actix_web::cookie::Cookie::build("access_token", t.clone())
                        .path("/")
                        .http_only(true)
                        .same_site(actix_web::cookie::SameSite::Lax)
                        .secure(is_secure)
                        .max_age(actix_web::cookie::time::Duration::days(7))
                        .finish();

                    Ok(LoginOutcome { cookie, access_token: t, user_id, username: db_username, role })
                }
                Err(e) => {
                    warn!("Failed to generate token: {:?}", e);
                    Err(HttpResponse::InternalServerError().json(serde_json::json!({
                        "status": "error",
                        "message": "Failed to generate token"
                    })))
                }
            }
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

// User logout endpoint (clears HttpOnly cookie)
#[utoipa::path(
    post,
    path = "/api/auth/logout",
    responses(
        (status = 200, description = "Logged out successfully", body = serde_json::Value)
    ),
    tag = "Authentication"
)]
#[post("/auth/logout")]
pub async fn user_logout() -> HttpResponse {
    let cookie = actix_web::cookie::Cookie::build("access_token", "")
        .path("/")
        .http_only(true)
        .same_site(actix_web::cookie::SameSite::Lax)
        .max_age(actix_web::cookie::time::Duration::ZERO)
        .finish();

    let mut response = HttpResponse::Ok();
    response.cookie(cookie);
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