use actix_web::{post, web, HttpRequest, HttpResponse};
use np2p::network::P2PService;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use crate::config::Config;
use crate::utils;
use crate::db::MainDbPool;
use crate::p2p_restore::restore_file_to_path;

#[utoipa::path(
    post,
    path = "/api/p2p/restore/{hash}",
    params(("hash" = String, Path, description = "File hash to restore from P2P backup")),
    responses(
        (status = 200, description = "Restored file as binary download", content_type = "application/octet-stream"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "File not found or no shards available"),
        (status = 500, description = "Restore failed")
    ),
    tag = "P2P"
)]
#[post("/p2p/restore/{hash}")]
pub async fn restore_p2p_file(
    req: HttpRequest,
    path: web::Path<String>,
    config: web::Data<Config>,
    p2p_service: web::Data<Arc<P2PService>>,
    pool: web::Data<MainDbPool>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "restore_p2p_file", config.get_api_key()).await {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };

    // Object-level authorization: only admins may restore arbitrary users' media.
    let owner_user_id: Option<String> = if claims.role == "admin" {
        None
    } else {
        Some(claims.user_id.clone())
    };

    let file_hash = path.into_inner();
    let api_secret = match config.get_api_key() {
        Ok(k) => k.to_string(),
        Err(e) => return Ok(HttpResponse::InternalServerError().body(e.to_string())),
    };

    struct TempFileGuard {
        path: PathBuf,
    }
    impl Drop for TempFileGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    let tmp_path = std::env::temp_dir().join(format!(
        "reminisce_http_restore_{}_{}.bin",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    ));
    let guard = TempFileGuard { path: tmp_path.clone() };

    match restore_file_to_path(&pool.0, &p2p_service, &file_hash, &api_secret, owner_user_id.as_deref(), &tmp_path).await {
        Ok(meta) => {
            let safe_base = std::path::Path::new(&meta.filename)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("restored.bin");
            let safe_name: String = safe_base.chars()
                .filter(|c| !matches!(c, '"' | '\r' | '\n' | '\\' | '/' | '\0'))
                .collect();
            let disposition = format!("attachment; filename=\"{}\"", safe_name);

            let file = match tokio::fs::File::open(&tmp_path).await {
                Ok(f) => f,
                Err(e) => return Ok(HttpResponse::InternalServerError().body(e.to_string())),
            };

            let stream = futures::stream::unfold((file, guard), |(mut file, guard)| async move {
                let mut buf = vec![0u8; 64 * 1024];
                match file.read(&mut buf).await {
                    Ok(0) => None,
                    Ok(n) => {
                        buf.truncate(n);
                        Some((Ok::<actix_web::web::Bytes, std::io::Error>(actix_web::web::Bytes::from(buf)), (file, guard)))
                    }
                    Err(e) => Some((Err(e), (file, guard))),
                }
            });

            Ok(HttpResponse::Ok()
                .content_type("application/octet-stream")
                .append_header(("Content-Disposition", disposition))
                .append_header(("Content-Length", meta.size_bytes.to_string()))
                .streaming(stream))
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found in database") || msg.contains("No shards found") {
                Ok(HttpResponse::NotFound().body(msg))
            } else {
                Ok(HttpResponse::InternalServerError().body(msg))
            }
        }
    }
}
