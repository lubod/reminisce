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

        // 3. Query parameter 'token'
        if token.is_none() {
            if let Ok(query) = web::Query::<std::collections::HashMap<String, String>>::from_query(req.query_string()) {
                if let Some(t) = query.get("token") {
                    token = Some(t.clone());
                }
            }
        }

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

            struct CachedUserStatus {
                role: String,
                is_active: bool,
                fetched_at: std::time::Instant,
            }
            static USER_CACHE: std::sync::OnceLock<std::sync::Arc<std::sync::Mutex<std::collections::HashMap<uuid::Uuid, CachedUserStatus>>>> = std::sync::OnceLock::new();
            let cache = USER_CACHE.get_or_init(|| std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())));

            let (role, is_active) = {
                let mut cache_guard = cache.lock().unwrap_or_else(|e| e.into_inner());

                // Evict stale entries if cache grows beyond 500 items
                if cache_guard.len() > 500 {
                    cache_guard.retain(|_, v| v.fetched_at.elapsed() < std::time::Duration::from_secs(60));
                }

                if let Some(cached) = cache_guard.get(&user_uuid) {
                    if cached.fetched_at.elapsed() < std::time::Duration::from_secs(5) {
                        Some((cached.role.clone(), cached.is_active))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }.unzip();

            let (user_role, user_active) = if let (Some(r), Some(a)) = (role, is_active) {
                (r, a)
            } else if let Some(pool) = pool {
                let client = pool.0.get().await.map_err(|e| {
                    log::error!("FromRequest DB connection error: {:?}", e);
                    actix_web::error::ErrorInternalServerError("Database connection failed")
                })?;
                let row = client.query_opt(
                    "SELECT role, is_active FROM users WHERE id = $1",
                    &[&user_uuid]
                ).await.map_err(|e| {
                    log::error!("FromRequest DB query error: {:?}", e);
                    actix_web::error::ErrorInternalServerError("Database error")
                })?;

                if let Some(row) = row {
                    let r: String = row.get("role");
                    let a: bool = row.get("is_active");
                    let mut cache_write = cache.lock().unwrap_or_else(|e| e.into_inner());
                    cache_write.insert(user_uuid, CachedUserStatus {
                        role: r.clone(),
                        is_active: a,
                        fetched_at: std::time::Instant::now(),
                    });
                    (r, a)
                } else {
                    return Err(actix_web::error::ErrorUnauthorized("User not found"));
                }
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

    let row = match client.query_one("SELECT COUNT(*) FROM users", &[]).await {
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
    match client.execute(
        "INSERT INTO users (username, email, password_hash, role) VALUES ($1, $2, $3, 'admin')",
        &[&req_body.username, &email, &password_hash],
    ).await {
        Ok(_) => {
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

    if req_body.username.trim().is_empty() || req_body.password.is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "status": "error",
            "message": "Username and password are required"
        }));
    }

    // Get database connection
    let client = match pool.0.get().await {
        Ok(client) => client,
        Err(e) => {
            warn!("Failed to get database connection: {:?}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "status": "error",
                "message": "Database connection failed"
            }));
        }
    };

    // Query user from database
    let query = "SELECT id, username, password_hash, role, is_active FROM users WHERE username = $1";

    let row = match instrumented_query_opt(&client, query, &[&req_body.username], "user_login_query").await {
        Ok(Some(row)) => row,
        Ok(None) => {
            warn!("User not found: {}", req_body.username);

            // Increment failed login metrics
            USER_LOGIN_FAILURES_TOTAL.inc();

            return HttpResponse::Unauthorized().json(serde_json::json!({
                "status": "error",
                "message": "Invalid username or password"
            }));
        }
        Err(e) => {
            warn!("Database error during login: {:?}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "status": "error",
                "message": "Login failed"
            }));
        }
    };

    let user_id: Uuid = row.get("id");
    let username: String = row.get("username");
    let password_hash: String = row.get("password_hash");
    let role: String = row.get("role");
    let is_active: bool = row.get("is_active");

    // Check if user is active
    if !is_active {
        warn!("Inactive user attempted login: {}", username);
        return HttpResponse::Unauthorized().json(serde_json::json!({
            "status": "error",
            "message": "Account is disabled"
        }));
    }

    // Verify password
    match verify_password(&req_body.password, &password_hash) {
        Ok(true) => {
            // Password is correct, generate JWT
            let expiration_time = chrono::Utc::now() + chrono::Duration::days(7);
            let claims = Claims {
                user_id: user_id.to_string(),
                username: username.clone(),
                email: String::new(),
                role: role.clone(),
                exp: expiration_time.timestamp() as usize,
                scope: None,
            };

            let api_key = match config.get_api_key() {
                Ok(k) => k,
                Err(e) => return HttpResponse::InternalServerError().json(serde_json::json!({
                    "status": "error",
                    "message": format!("Configuration error: {}", e)
                })),
            };

            let token = encode(
                &Header::new(Algorithm::HS512),
                &claims,
                &EncodingKey::from_secret(api_key.as_bytes())
            );

            match token {
                Ok(t) => {
                    // Update last_login_at
                    let _ = instrumented_execute(
                        &client,
                        "UPDATE users SET last_login_at = NOW() WHERE id = $1",
                        &[&user_id],
                        "update_last_login"
                    ).await;

                    info!("User logged in successfully: {}", username);

                    // Increment successful login metrics
                    USER_LOGINS_TOTAL.inc();

                    let is_secure = config.environment.as_deref() != Some("development") && config.environment.as_deref() != Some("dev");
                    let cookie = actix_web::cookie::Cookie::build("access_token", t.clone())
                        .path("/")
                        .http_only(true)
                        .same_site(actix_web::cookie::SameSite::Lax)
                        .secure(is_secure)
                        .max_age(actix_web::cookie::time::Duration::days(7))
                        .finish();

                    let image_token_exp = chrono::Utc::now() + chrono::Duration::hours(24);
                    let image_claims = Claims {
                        user_id: user_id.to_string(),
                        username: username.clone(),
                        email: String::new(),
                        role: role.clone(),
                        exp: image_token_exp.timestamp() as usize,
                        scope: Some("media_read".to_string()),
                    };
                    let image_token = encode(
                        &Header::new(Algorithm::HS512),
                        &image_claims,
                        &EncodingKey::from_secret(api_key.as_bytes())
                    ).unwrap_or_default();

                    let mut response = HttpResponse::Ok();
                    response.cookie(cookie);
                    response.json(serde_json::json!({
                        "access_token": t,
                        "image_token": image_token,
                        "user": {
                            "id": user_id.to_string(),
                            "username": username,
                            "role": role
                        }
                    }))
                }
                Err(e) => {
                    warn!("Failed to generate token: {:?}", e);
                    HttpResponse::InternalServerError().json(serde_json::json!({
                        "status": "error",
                        "message": "Failed to generate token"
                    }))
                }
            }
        }
        Ok(false) => {
            warn!("Invalid password for user: {}", username);

            // Increment failed login metrics
            USER_LOGIN_FAILURES_TOTAL.inc();

            HttpResponse::Unauthorized().json(serde_json::json!({
                "status": "error",
                "message": "Invalid username or password"
            }))
        }
        Err(e) => {
            warn!("Password verification error: {:?}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "status": "error",
                "message": "Authentication failed"
            }))
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