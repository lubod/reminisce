use actix_web::{post, web, HttpRequest, HttpResponse};
use np2p::network::P2PService;
use std::sync::Arc;
use crate::config::Config;
use crate::utils;
use crate::db::MainDbPool;
use crate::p2p_restore::restore_file;

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
    let _claims = match utils::authenticate_request(&req, "restore_p2p_file", config.get_api_key()) {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };

    let file_hash = path.into_inner();
    match restore_file(&pool.0, &p2p_service, &file_hash).await {
        Ok(restored) => {
            let disposition = format!("attachment; filename=\"{}\"", restored.filename);
            Ok(HttpResponse::Ok()
                .content_type("application/octet-stream")
                .append_header(("Content-Disposition", disposition))
                .body(restored.data))
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
