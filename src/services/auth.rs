use actix_web::{ get, post, web, HttpResponse };
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
use crate::db_instrumentation::{instrumented_query_one, instrumented_query_opt, instrumented_execute};

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
}

use actix_web::{FromRequest, dev::Payload, Error as ActixError};
use futures_util::future::{Ready, ok, err};

impl FromRequest for Claims {
    type Error = ActixError;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &actix_web::HttpRequest, _payload: &mut Payload) -> Self::Future {
        // Get the API secret from Config app data
        let secret = if let Some(config) = req.app_data::<web::Data<Config>>() {
            config.api_secret_key.clone().unwrap_or_default()
        } else {
            return err(actix_web::error::ErrorInternalServerError("Config not available"));
        };

        // Extract token from Authorization header or query parameter
        let mut token = None;

        if let Some(auth_header) = req.headers().get("Authorization") {
            if let Ok(auth_str) = auth_header.to_str() {
                if auth_str.starts_with("Bearer ") {
                    token = Some(auth_str.trim_start_matches("Bearer ").to_string());
                }
            }
        }

        if token.is_none() {
            if let Ok(query) = web::Query::<std::collections::HashMap<String, String>>::from_query(req.query_string()) {
                if let Some(t) = query.get("token") {
                    token = Some(t.clone());
                }
            }
        }

        match token {
            Some(token_str) => {
                let validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS512);
                match jsonwebtoken::decode::<Claims>(
                    &token_str,
                    &jsonwebtoken::DecodingKey::from_secret(secret.as_ref()),
                    &validation,
                ) {
                    Ok(token_data) => ok(token_data.claims),
                    Err(_) => err(actix_web::error::ErrorUnauthorized("Invalid token")),
                }
            }
            None => err(actix_web::error::ErrorUnauthorized("Authentication required")),
        }
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
    pub username: String,
    pub password: String,
}

// Public registration is disabled — users are created by admins only.
#[post("/auth/register")]
pub async fn register_user() -> HttpResponse {
    HttpResponse::Forbidden().json(serde_json::json!({
        "status": "error",
        "message": "Registration is disabled. Contact your administrator."
    }))
}

// --- Setup endpoints (first-run only) ---

#[derive(Serialize, Deserialize)]
pub struct SetupRequest {
    pub username: String,
    pub password: String,
}

/// Returns whether the server needs initial setup (no users exist yet).
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

    let row = client.query_one("SELECT COUNT(*) FROM users", &[]).await.unwrap();
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
            };

            let token = encode(
                &Header::new(Algorithm::HS512),
                &claims,
                &EncodingKey::from_secret(config.get_api_key().as_bytes())
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

                    HttpResponse::Ok().json(serde_json::json!({
                        "access_token": t,
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