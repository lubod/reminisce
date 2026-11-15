//! One-off maintenance: repair images.width/height that were polluted by the
//! quality scorer storing its 384px working-thumbnail dimensions instead of
//! the original image dimensions.

use actix_web::{post, web, HttpRequest, HttpResponse};
use std::sync::atomic::{AtomicBool, Ordering};

static BACKFILL_RUNNING: AtomicBool = AtomicBool::new(false);

/// Header-decode the image and return its true pixel dimensions.
fn header_dims(path: &std::path::Path) -> Option<(i32, i32)> {
    let f = std::fs::File::open(path).ok()?;
    let reader = image::io::Reader::new(std::io::BufReader::new(f))
        .with_guessed_format()
        .ok()?;
    let (w, h) = reader.into_dimensions().ok()?;
    if w == 0 || h == 0 {
        return None;
    }
    Some((w as i32, h as i32))
}

#[utoipa::path(
    post,
    path = "/admin/backfill-dimensions",
    responses(
        (status = 202, description = "Backfill started"),
        (status = 401), (status = 403), (status = 409, description = "Already running")
    ),
    tag = "Admin"
)]
#[post("/admin/backfill-dimensions")]
pub async fn trigger_dimension_backfill(
    req: HttpRequest,
    config: web::Data<crate::config::Config>,
    pool: web::Data<crate::db::MainDbPool>,
) -> HttpResponse {
    let claims = match crate::utils::authenticate_request(&req, "trigger_dimension_backfill", config.get_api_key()).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(serde_json::json!({"error": "admin required"}));
    }

    if BACKFILL_RUNNING
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return HttpResponse::Conflict().json(serde_json::json!({"error": "backfill already running"}));
    }

    let images_dir = config.get_images_dir().to_string();
    tokio::spawn(async move {
        const BATCH: i64 = 1000;
        let mut cursor: Option<(chrono::DateTime<chrono::Utc>, String)> = None;
        let mut checked: u64 = 0;
        let mut fixed: u64 = 0;
        let mut skipped: u64 = 0;
        loop {
            let rows = match pool.0.get().await {
                Ok(c) => match &cursor {
                    None => {
                        c.query(
                            "SELECT hash, ext, width, height, created_at FROM images \
                             WHERE deleted_at IS NULL \
                             ORDER BY created_at ASC, hash ASC LIMIT $1",
                            &[&BATCH],
                        )
                        .await
                    }
                    Some((last_ts, last_hash)) => {
                        c.query(
                            "SELECT hash, ext, width, height, created_at FROM images \
                             WHERE deleted_at IS NULL \
                               AND (created_at, hash) > ($1, $2) \
                             ORDER BY created_at ASC, hash ASC LIMIT $3",
                            &[last_ts, last_hash, &BATCH],
                        )
                        .await
                    }
                },
                Err(e) => { log::error!("[DIM-BACKFILL] pool error: {}", e); break; }
            };
            let rows = match rows {
                Ok(r) => r,
                Err(e) => { log::error!("[DIM-BACKFILL] query error: {}", e); break; }
            };
            if rows.is_empty() {
                break;
            }
            for row in &rows {
                let hash: String = row.get(0);
                let ext: String = row.get(1);
                let stored_w: Option<i32> = row.get(2);
                let stored_h: Option<i32> = row.get(3);
                let created_at: chrono::DateTime<chrono::Utc> = row.get(4);
                cursor = Some((created_at, hash.clone()));
                checked += 1;

                let path = match crate::media_utils::safe_resolve_content_path(&images_dir, &hash, &ext.to_lowercase()) {
                    Ok(p) => p,
                    Err(_) => { skipped += 1; continue; }
                };
                let dims = tokio::task::spawn_blocking(move || header_dims(&path))
                    .await
                    .ok()
                    .flatten();
                let Some((w, h)) = dims else { skipped += 1; continue };

                // Skip formats the decoder cannot really open (heic etc. would
                // have failed above already).
                if Some((w, h)) != (stored_w.zip(stored_h)) {
                    if let Err(e) = client_update(&pool, &hash, w, h).await {
                        log::error!("[DIM-BACKFILL] update failed for {}: {}", hash, e);
                        continue;
                    }
                    fixed += 1;
                }
            }
            log::info!("[DIM-BACKFILL] progress: checked={} fixed={} skipped={}", checked, fixed, skipped);
            tokio::task::yield_now().await;
        }
        BACKFILL_RUNNING.store(false, Ordering::Release);
        log::info!("[DIM-BACKFILL] complete: checked={} fixed={} skipped={}", checked, fixed, skipped);
    });

    HttpResponse::Accepted().json(serde_json::json!({"status": "dimension backfill started"}))
}

async fn client_update(pool: &web::Data<crate::db::MainDbPool>, hash: &str, w: i32, h: i32) -> Result<(), String> {
    let c = pool.0.get().await.map_err(|e| e.to_string())?;
    c.execute("UPDATE images SET width=$1, height=$2 WHERE hash=$3", &[&w, &h, &hash])
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{http, test as actix_test, App};
    use image::ImageEncoder;
    use serial_test::serial;

    fn test_db_env_available() -> bool {
        std::env::var("TEST_DATABASE_URL").is_ok()
            || std::env::var("PGHOST").is_ok()
            || std::env::var("PGPORT").is_ok()
    }

    fn make_test_jpeg_file(path: &std::path::Path, w: u32, h: u32) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([10, 120, 220]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::codecs::jpeg::JpegEncoder::new(&mut buf)
            .write_image(img.as_raw(), w, h, image::ColorType::Rgb8)
            .unwrap();
        std::fs::write(path, buf.into_inner()).unwrap();
    }

    fn mini_config(images_dir: String) -> crate::config::Config {
        let mut c: crate::config::Config = serde_yaml::from_str(
            "api_secret_key: \"test-secret-for-unit-tests-0123456789abcdef01234567\"\n",
        )
        .expect("minimal config");
        c.images_dir = Some(images_dir);
        c
    }

    fn sign_jwt(user_id: &str, role: &str, secret: &str) -> String {
        let exp = (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize;
        let claims = crate::services::auth::Claims {
            user_id: user_id.to_string(),
            username: format!("user-{}", role),
            email: format!("{}@example.com", role),
            role: role.to_string(),
            exp,
            scope: None,
        };
        jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS512),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap()
    }

    #[test]
    fn header_dims_reads_valid_png_and_jpeg() {
        let base = std::env::temp_dir().join(format!("rem_maint_unit_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();

        let jpg_path = base.join("sample.jpg");
        make_test_jpeg_file(&jpg_path, 120, 80);
        assert_eq!(header_dims(&jpg_path), Some((120, 80)));

        let png_path = base.join("sample.png");
        let img = image::RgbImage::from_pixel(64, 48, image::Rgb([50, 60, 70]));
        image::DynamicImage::ImageRgb8(img)
            .save(&png_path)
            .unwrap();
        assert_eq!(header_dims(&png_path), Some((64, 48)));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn header_dims_returns_none_for_missing_or_corrupt_file() {
        let base = std::env::temp_dir().join(format!("rem_maint_bad_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();

        let missing = base.join("does_not_exist.jpg");
        assert_eq!(header_dims(&missing), None);

        let corrupt = base.join("corrupt.jpg");
        std::fs::write(&corrupt, b"not an image").unwrap();
        assert_eq!(header_dims(&corrupt), None);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[actix_web::test]
    #[serial]
    async fn backfill_dimensions_rejects_unauthenticated_and_non_admin() {
        if !test_db_env_available() {
            eprintln!("backfill_dimensions_rejects_unauthenticated_and_non_admin: skipping (no dev PG env)");
            return;
        }
        let (pool, _db) = crate::test_utils::setup_test_database_with_instance().await;
        let base = std::env::temp_dir().join(format!("rem_maint_auth_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        let cfg = mini_config(base.to_string_lossy().to_string());
        let secret = cfg.get_api_key().unwrap().to_string();

        // Insert a regular (non-admin) user into DB since authenticate_request reads role from DB.
        let non_admin_id = uuid::Uuid::new_v4();
        let client = pool.get().await.unwrap();
        client
            .execute(
                "INSERT INTO users (id, username, email, password_hash, role) VALUES ($1, $2, $3, 'h', 'user')",
                &[&non_admin_id, &format!("u_{}", non_admin_id), &format!("{}@example.com", non_admin_id)],
            )
            .await
            .unwrap();

        let app = actix_test::init_service(
            App::new()
                .app_data(web::Data::new(crate::db::MainDbPool(pool)))
                .app_data(web::Data::new(cfg))
                .service(trigger_dimension_backfill),
        )
        .await;

        let unauthed = actix_test::TestRequest::post()
            .uri("/admin/backfill-dimensions")
            .to_request();
        let resp = actix_test::call_service(&app, unauthed).await;
        assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);

        let user_token = sign_jwt(&non_admin_id.to_string(), "user", &secret);
        let non_admin_req = actix_test::TestRequest::post()
            .uri("/admin/backfill-dimensions")
            .insert_header(("Authorization", format!("Bearer {}", user_token)))
            .to_request();
        let resp = actix_test::call_service(&app, non_admin_req).await;
        assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[actix_web::test]
    #[serial]
    async fn backfill_dimensions_repairs_corrupted_dimensions_for_admin() {
        if !test_db_env_available() {
            eprintln!("backfill_dimensions_repairs_corrupted_dimensions_for_admin: skipping (no dev PG env)");
            return;
        }
        // Ensure prior backfill task (if any) is finished before resetting/testing.
        for _ in 0..50 {
            if !BACKFILL_RUNNING.load(Ordering::Acquire) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        let (pool, _db) = crate::test_utils::setup_test_database_with_instance().await;
        let base = std::env::temp_dir().join(format!("rem_maint_repair_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        let cfg = mini_config(base.to_string_lossy().to_string());
        let secret = cfg.get_api_key().unwrap().to_string();

        let admin_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let hash = "a".repeat(64);
        let img_path = base.join(&hash[..2]).join(format!("{}.jpg", hash));
        make_test_jpeg_file(&img_path, 160, 90);

        let client = pool.get().await.unwrap();
        client
            .execute(
                "INSERT INTO images (user_id, deviceid, hash, name, ext, width, height) \
                 VALUES ($1, 'dev1', $2, 'photo.jpg', 'jpg', 384, 384)",
                &[&admin_id, &hash],
            )
            .await
            .unwrap();

        let app = actix_test::init_service(
            App::new()
                .app_data(web::Data::new(crate::db::MainDbPool(pool.clone())))
                .app_data(web::Data::new(cfg))
                .service(trigger_dimension_backfill),
        )
        .await;

        let admin_token = sign_jwt(&admin_id.to_string(), "admin", &secret);
        let req = actix_test::TestRequest::post()
            .uri("/admin/backfill-dimensions")
            .insert_header(("Authorization", format!("Bearer {}", admin_token)))
            .to_request();
        let resp = actix_test::call_service(&app, req).await;
        assert_eq!(resp.status(), http::StatusCode::ACCEPTED);

        for _ in 0..100 {
            if !BACKFILL_RUNNING.load(Ordering::Acquire) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(!BACKFILL_RUNNING.load(Ordering::Acquire), "backfill did not finish in time");

        let row = client
            .query_one("SELECT width, height FROM images WHERE hash = $1", &[&hash])
            .await
            .unwrap();
        let width: Option<i32> = row.get(0);
        let height: Option<i32> = row.get(1);
        assert_eq!((width, height), (Some(160), Some(90)));

        let _ = std::fs::remove_dir_all(&base);
    }
}
