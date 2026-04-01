use actix_web::{ get, web, HttpRequest, HttpResponse };
use log::error;
use serde::{ Deserialize, Serialize };
use utoipa::{ IntoParams, ToSchema };

use crate::config::Config;
use crate::utils;
use crate::db::MainDbPool;

#[derive(Deserialize, Debug, Clone, ToSchema, IntoParams)]
#[schema(example = json!({
    "hash": "somehash",
    "device_id": "my_device"
}))]
pub struct ImageCheckQuery {
    hash: String,
    #[serde(default = "default_device_id")]
    #[allow(dead_code)]
    device_id: String,
}

#[derive(Deserialize, Debug, Clone, ToSchema, IntoParams)]
#[schema(example = json!({
    "hash": "somehash",
    "device_id": "my_device"
}))]
pub struct VideoCheckQuery {
    hash: String,
    #[serde(default = "default_device_id")]
    #[allow(dead_code)]
    device_id: String,
}

fn default_device_id() -> String {
    "web-client".to_string()
}

#[derive(Serialize, ToSchema)]
#[schema(example = json!({
    "exists_for_deviceid": true,
    "exists": true
}))]
pub struct ExistenceResponse {
    exists_without_deviceid: bool,
    exists: bool,
}

#[utoipa::path(
    get,
    path = "/check_image_exists",
    params(ImageCheckQuery),
    responses(
        (status = 200, description = "Check successful", body = ExistenceResponse),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    )
)]
#[get("/check_image_exists")]
pub async fn check_image_exists(
    req: HttpRequest,
    query: web::Query<ImageCheckQuery>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>
) -> HttpResponse {
    let claims = match
        utils::authenticate_request(&req, "check_image_exists", config.get_api_key())
    {
        Ok(claims) => claims,
        Err(response) => {
            return response;
        }
    };

    let user_uuid = match utils::parse_user_uuid(&claims.user_id) {
        Ok(u) => u,
        Err(_) => return HttpResponse::Unauthorized().finish(),
    };

    let hash_to_find = &query.hash;
    let check_result = match utils::check_if_exists(hash_to_find, &user_uuid, "images", pool).await {
        Ok(result) => result,
        Err(e) => {
            error!("Failed to check image existence: {}", e);
            return HttpResponse::InternalServerError().json("Failed to check image existence");
        }
    };
    let response_data = ExistenceResponse {
        exists_without_deviceid: check_result.exists_verified,
        exists: check_result.exists_for_user
    };
    HttpResponse::Ok().json(response_data)
}

#[utoipa::path(
    get,
    path = "/check_video_exists",
    params(VideoCheckQuery),
    responses(
        (status = 200, description = "Check successful", body = ExistenceResponse),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    )
)]
#[get("/check_video_exists")]
pub async fn check_video_exists(
    req: HttpRequest,
    query: web::Query<VideoCheckQuery>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>
) -> HttpResponse {
    let claims = match
        utils::authenticate_request(&req, "check_video_exists", config.get_api_key())
    {
        Ok(claims) => claims,
        Err(response) => {
            return response;
        }
    };

    let user_uuid = match utils::parse_user_uuid(&claims.user_id) {
        Ok(u) => u,
        Err(_) => return HttpResponse::Unauthorized().finish(),
    };

    let hash_to_find = &query.hash;
    let check_result = match utils::check_if_exists(hash_to_find, &user_uuid, "videos", pool).await {
        Ok(result) => result,
        Err(e) => {
            error!("Failed to check video existence: {}", e);
            return HttpResponse::InternalServerError().json("Failed to check video existence");
        }
    };
    let response_data = ExistenceResponse {
        exists_without_deviceid: check_result.exists_verified,
        exists: check_result.exists_for_user
    };
    HttpResponse::Ok().json(response_data)
}