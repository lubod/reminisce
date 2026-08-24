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
        let mut offset: i64 = 0;
        let mut checked: u64 = 0;
        let mut fixed: u64 = 0;
        let mut skipped: u64 = 0;
        loop {
            let rows = match pool.0.get().await {
                Ok(c) => c.query(
                    "SELECT hash, ext, width, height FROM images \
                     WHERE deleted_at IS NULL \
                     ORDER BY created_at, hash LIMIT $1 OFFSET $2",
                    &[&BATCH, &offset],
                ).await,
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
                checked += 1;

                let path = match crate::media_utils::safe_resolve_content_path(&images_dir, &hash, &ext.to_lowercase()) {
                    Ok(p) => p,
                    Err(_) => { skipped += 1; continue; }
                };
                let Some((w, h)) = header_dims(&path) else { skipped += 1; continue };

                // Skip formats the decoder cannot really open (heic etc. would
                // have failed above already).
                if Some((w, h)) != (stored_w.zip(stored_h)) {
                    if let Err(e) = client_update(&pool, &hash, w, h).await {
                        log::error!("[DIM-BACKFILL] update failed for {}: {}", hash, e);
                        continue;
                    }
                    fixed += 1;
                }
                let _ = &mut offset;
            }
            offset += BATCH;
            log::info!("[DIM-BACKFILL] progress: checked={} fixed={} skipped={} offset={}", checked, fixed, skipped, offset);
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
