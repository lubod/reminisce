use actix_web::{delete, get, patch, post, web, HttpResponse};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth_utils::hash_password;
use crate::db::MainDbPool;
use crate::services::auth::Claims;

#[derive(Serialize)]
struct UserRecord {
    id: String,
    username: String,
    email: String,
    role: String,
    is_active: bool,
    created_at: String,
    last_login_at: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub role: Option<String>, // "user", "admin", "viewer" — defaults to "user"
}

#[derive(Deserialize)]
pub struct UpdateUserRequest {
    pub role: Option<String>,
    pub is_active: Option<bool>,
    pub password: Option<String>,
}

/// List all users — admin only.
#[get("/users")]
pub async fn list_users(claims: Claims, pool: web::Data<MainDbPool>) -> HttpResponse {
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(serde_json::json!({"status":"error","message":"Admin only"}));
    }

    let client = match pool.0.get().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError().finish(),
    };

    let rows = match client.query(
        "SELECT id, username, email, role, is_active, created_at, last_login_at FROM users ORDER BY created_at ASC",
        &[],
    ).await {
        Ok(r) => r,
        Err(e) => {
            warn!("list_users DB error: {}", e);
            return HttpResponse::InternalServerError().finish();
        }
    };

    let users: Vec<UserRecord> = rows.iter().map(|row| {
        let id: Uuid = row.get("id");
        let created_at: chrono::DateTime<chrono::Utc> = row.get("created_at");
        let last_login_at: Option<chrono::DateTime<chrono::Utc>> = row.get("last_login_at");
        UserRecord {
            id: id.to_string(),
            username: row.get("username"),
            email: row.get("email"),
            role: row.get("role"),
            is_active: row.get("is_active"),
            created_at: created_at.to_rfc3339(),
            last_login_at: last_login_at.map(|t| t.to_rfc3339()),
        }
    }).collect();

    HttpResponse::Ok().json(users)
}

/// Create a user — admin only.
#[post("/users")]
pub async fn create_user(
    claims: Claims,
    pool: web::Data<MainDbPool>,
    body: web::Json<CreateUserRequest>,
) -> HttpResponse {
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(serde_json::json!({"status":"error","message":"Admin only"}));
    }

    if body.username.len() < 3 || body.password.len() < 8 {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "status": "error",
            "message": "Username must be ≥3 chars and password ≥8 chars"
        }));
    }

    let role = body.role.clone().unwrap_or_else(|| "user".to_string());
    if !["admin", "user", "viewer"].contains(&role.as_str()) {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "status": "error",
            "message": "Role must be admin, user, or viewer"
        }));
    }

    let password_hash = match hash_password(&body.password) {
        Ok(h) => h,
        Err(_) => return HttpResponse::InternalServerError().finish(),
    };

    let email = format!("{}@local", body.username);

    let client = match pool.0.get().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError().finish(),
    };

    match client.query_one(
        "INSERT INTO users (username, email, password_hash, role) VALUES ($1, $2, $3, $4) RETURNING id",
        &[&body.username, &email, &password_hash, &role],
    ).await {
        Ok(row) => {
            let id: Uuid = row.get(0);
            info!("Admin {} created user: {} ({})", claims.username, body.username, role);
            HttpResponse::Created().json(serde_json::json!({
                "status": "ok",
                "user_id": id.to_string()
            }))
        }
        Err(e) => {
            if e.to_string().contains("duplicate key") {
                HttpResponse::BadRequest().json(serde_json::json!({
                    "status": "error",
                    "message": "Username already exists"
                }))
            } else {
                warn!("create_user DB error: {}", e);
                HttpResponse::InternalServerError().finish()
            }
        }
    }
}

/// Update a user's role, active status, or password — admin only.
/// An admin cannot deactivate or demote themselves.
#[patch("/users/{id}")]
pub async fn update_user(
    claims: Claims,
    pool: web::Data<MainDbPool>,
    path: web::Path<String>,
    body: web::Json<UpdateUserRequest>,
) -> HttpResponse {
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(serde_json::json!({"status":"error","message":"Admin only"}));
    }

    let target_id = match Uuid::parse_str(&path) {
        Ok(id) => id,
        Err(_) => return HttpResponse::BadRequest().json(serde_json::json!({"status":"error","message":"Invalid user ID"})),
    };

    // Prevent admin from demoting/deactivating themselves
    if target_id.to_string() == claims.user_id {
        if body.role.as_deref() == Some("user") || body.role.as_deref() == Some("viewer") {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "status": "error", "message": "Cannot remove your own admin role"
            }));
        }
        if body.is_active == Some(false) {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "status": "error", "message": "Cannot deactivate your own account"
            }));
        }
    }

    let client = match pool.0.get().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError().finish(),
    };

    if let Some(ref role) = body.role {
        if !["admin", "user", "viewer"].contains(&role.as_str()) {
            return HttpResponse::BadRequest().json(serde_json::json!({"status":"error","message":"Invalid role"}));
        }
        let _ = client.execute("UPDATE users SET role = $1 WHERE id = $2", &[role, &target_id]).await;
    }

    if let Some(active) = body.is_active {
        let _ = client.execute("UPDATE users SET is_active = $1 WHERE id = $2", &[&active, &target_id]).await;
    }

    if let Some(ref new_password) = body.password {
        if new_password.len() < 8 {
            return HttpResponse::BadRequest().json(serde_json::json!({"status":"error","message":"Password must be ≥8 chars"}));
        }
        match hash_password(new_password) {
            Ok(hash) => { let _ = client.execute("UPDATE users SET password_hash = $1 WHERE id = $2", &[&hash, &target_id]).await; }
            Err(_) => return HttpResponse::InternalServerError().finish(),
        }
    }

    info!("Admin {} updated user {}", claims.username, target_id);
    HttpResponse::Ok().json(serde_json::json!({"status": "ok"}))
}

/// Delete a user — admin only. Cannot delete yourself.
#[delete("/users/{id}")]
pub async fn delete_user(
    claims: Claims,
    pool: web::Data<MainDbPool>,
    path: web::Path<String>,
) -> HttpResponse {
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(serde_json::json!({"status":"error","message":"Admin only"}));
    }

    let target_id = match Uuid::parse_str(&path) {
        Ok(id) => id,
        Err(_) => return HttpResponse::BadRequest().json(serde_json::json!({"status":"error","message":"Invalid user ID"})),
    };

    if target_id.to_string() == claims.user_id {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "status": "error", "message": "Cannot delete your own account"
        }));
    }

    let client = match pool.0.get().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError().finish(),
    };

    match client.execute("DELETE FROM users WHERE id = $1", &[&target_id]).await {
        Ok(n) if n > 0 => {
            info!("Admin {} deleted user {}", claims.username, target_id);
            HttpResponse::Ok().json(serde_json::json!({"status": "ok"}))
        }
        Ok(_) => HttpResponse::NotFound().json(serde_json::json!({"status":"error","message":"User not found"})),
        Err(e) => {
            warn!("delete_user DB error: {}", e);
            HttpResponse::InternalServerError().finish()
        }
    }
}
