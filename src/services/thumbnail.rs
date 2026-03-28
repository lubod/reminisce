use actix_web::{ get, web, HttpRequest, HttpResponse };
use chrono::{ DateTime, Utc };
use log::{ error, info, warn };

use serde::{ Deserialize, Serialize };
use std::time::Instant;
use tokio::fs as async_fs;
use utoipa::{ IntoParams, ToSchema };
use image::GenericImageView;
use std::io::Cursor;

use crate::config::Config;
use crate::metrics::{THUMBNAIL_DURATION, THUMBNAIL_SUCCESS_TOTAL, THUMBNAIL_FAILURES_TOTAL};
use crate::utils;
use crate::db::MainDbPool;

#[utoipa::path(
    get,
    path = "/face/{face_id}/thumbnail",
    params(
        ("face_id" = i64, Path, description = "Face ID")
    ),
    responses(
        (status = 200, description = "Face thumbnail found"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Face or image not found"),
        (status = 500, description = "Internal server error")
    )
)]
#[get("/face/{face_id}/thumbnail")]
pub async fn get_face_thumbnail(
    req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match
        utils::authenticate_request(
            &req,
            "get_face_thumbnail",
            config.get_api_key()
        )
    {
        Ok(claims) => claims,
        Err(response) => {
            return Ok(response);
        }
    };

    let face_id = path.into_inner();
    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;
    
    // Define cache path
    let faces_dir = std::path::Path::new(config.get_images_dir()).join("faces");
    if !async_fs::try_exists(&faces_dir).await.unwrap_or(false) {
        if let Err(e) = async_fs::create_dir_all(&faces_dir).await {
            error!("Failed to create faces cache directory: {}", e);
        }
    }
    
    let cache_path = faces_dir.join(format!("{}.jpg", face_id));

    // 1. Check cache first
    if async_fs::try_exists(&cache_path).await.unwrap_or(false) {
        match async_fs::read(&cache_path).await {
            Ok(data) => {
                return Ok(HttpResponse::Ok()
                    .content_type("image/jpeg")
                    .insert_header(("Cache-Control", "public, max-age=31536000, immutable"))
                    .body(data));
            }
            Err(e) => {
                warn!("Failed to read cached face thumbnail {:?}: {}", cache_path, e);
                // Fallthrough to regenerate
            }
        }
    }

    let client = utils::get_db_client(&pool.0).await?;

    // Query face details and check if parent image is not deleted
    let row = client
        .query_opt(
            "SELECT f.image_hash, f.image_deviceid, f.bbox_x, f.bbox_y, f.bbox_width, f.bbox_height
             FROM faces f
             JOIN images i ON f.image_hash = i.hash AND f.image_deviceid = i.deviceid
             WHERE f.id = $1 AND i.deleted_at IS NULL AND (i.user_id = $2 OR $3 = 'admin')",
            &[&face_id, &user_uuid, &claims.role]
        )
        .await
        .map_err(|e| {
            error!("Failed to query face: {}", e);
            actix_web::error::ErrorInternalServerError("Database error")
        })?;

    if let Some(row) = row {
        let image_hash: String = row.get(0);
        let _device_id: String = row.get(1); 
        let x: i32 = row.get(2);
        let y: i32 = row.get(3);
        let w: i32 = row.get(4);
        let h: i32 = row.get(5);

        // Find the image file
        let sub_dir_path = utils::get_subdirectory_path(config.get_images_dir(), &image_hash);
        
        // Query extension from images table
        let ext_row = client.query_opt("SELECT ext FROM images WHERE hash = $1", &[&image_hash]).await.unwrap_or(None);
        let ext = if let Some(r) = ext_row { r.get::<_, String>(0) } else { "jpg".to_string() };
        
        let filename = format!("{}.{}", image_hash, ext);
        let image_path = sub_dir_path.join(&filename);

        if !async_fs::try_exists(&image_path).await.unwrap_or(false) {
             return Ok(
                HttpResponse::NotFound().json(
                    serde_json::json!({"status": "error", "message": "Original image not found."})
                )
            );
        }

        // Load image using the blocking image crate (in a blocking task)
        let image_path_clone = image_path.clone();
        let cache_path_clone = cache_path.clone();
        
        info!("Generating face thumbnail for face_id {}. Loading image: {:?}", face_id, image_path_clone);

        let face_img = web::block(move || {
            let mut img = image::open(&image_path_clone).map_err(|e| {
                error!("Failed to open image {:?}: {}", image_path_clone, e);
                e
            })?;

            // Apply EXIF orientation to full image FIRST before cropping
            // Face detection runs on oriented images, so bbox coordinates are in oriented space
            let file = std::fs::File::open(&image_path_clone).ok();
            if let Some(f) = file {
                let mut bufreader = std::io::BufReader::new(&f);
                let exifreader = kamadak_exif::Reader::new();
                if let Ok(exif) = exifreader.read_from_container(&mut bufreader) {
                    if let Some(field) = exif.get_field(kamadak_exif::Tag::Orientation, kamadak_exif::In::PRIMARY) {
                        if let kamadak_exif::Value::Short(ref v) = field.value {
                            if let Some(&orientation) = v.first() {
                                match orientation {
                                    1 => {}, // Normal
                                    2 => img = img.fliph(),
                                    3 => img = img.rotate180(),
                                    4 => img = img.flipv(),
                                    5 => { img = img.rotate90(); img = img.fliph(); },
                                    6 => img = img.rotate90(),
                                    7 => { img = img.rotate270(); img = img.fliph(); },
                                    8 => img = img.rotate270(),
                                    _ => {},
                                }
                            }
                        }
                    }
                }
            }

            // Validate crop bounds against the ORIENTED image dimensions
            let (img_w, img_h) = img.dimensions();
            let crop_x = (x.max(0) as u32).min(img_w - 1);
            let crop_y = (y.max(0) as u32).min(img_h - 1);
            let mut crop_w = w.max(1) as u32;
            let mut crop_h = h.max(1) as u32;

            // Ensure we don't go out of bounds
            if crop_x + crop_w > img_w { crop_w = img_w - crop_x; }
            if crop_y + crop_h > img_h { crop_h = img_h - crop_y; }

            if crop_w == 0 || crop_h == 0 {
                error!("Invalid crop dimensions for face {}: w={}, h={}", face_id, crop_w, crop_h);
                return Err(image::ImageError::Parameter(image::error::ParameterError::from_kind(image::error::ParameterErrorKind::DimensionMismatch)));
            }

            // Crop from the correctly oriented image - no need to apply orientation again
            let cropped = img.view(crop_x, crop_y, crop_w, crop_h).to_image();
            
            // Resize to thumbnail size (e.g. 200px)
            let resized = image::imageops::resize(&cropped, 200, 200, image::imageops::FilterType::Lanczos3);
            
            // Save to buffer
            let mut buffer = Cursor::new(Vec::new());
            resized.write_to(&mut buffer, image::ImageOutputFormat::Jpeg(80)).map_err(|e| {
                error!("Failed to encode face thumbnail: {}", e);
                e
            })?;
            
            let data = buffer.into_inner();
            
            // Save to cache (ignore errors)
            if let Err(e) = std::fs::write(&cache_path_clone, &data) {
                error!("Failed to write face thumbnail cache {:?}: {}", cache_path_clone, e);
            }
            
            Ok::<Vec<u8>, image::ImageError>(data)
        }).await.map_err(|e| {
             error!("Blocking task failed/panicked for face {}: {}", face_id, e);
             actix_web::error::ErrorInternalServerError("Image processing failed")
        })?.map_err(|e| {
             error!("Image processing error for face {}: {}", face_id, e);
             actix_web::error::ErrorInternalServerError("Failed to process image")
        })?;

        Ok(HttpResponse::Ok()
            .content_type("image/jpeg")
            .insert_header(("Cache-Control", "public, max-age=31536000, immutable"))
            .body(face_img))

    } else {
        Ok(
            HttpResponse::NotFound().json(
                serde_json::json!({"status": "error", "message": "Face not found."})
            )
        )
    }
}

/// Generate a thumbnail for an image file
///
/// # Arguments
/// * `image_path` - Path to the source image file
/// * `output_path` - Path where the thumbnail should be saved
/// * `max_dimension` - Maximum width or height for the thumbnail (aspect ratio preserved)
///
/// # Returns
/// * `Ok(())` if thumbnail generation succeeded
/// * `Err(...)` if thumbnail generation failed
pub async fn generate_thumbnail_for_image(
    image_path: &std::path::Path,
    output_path: &std::path::Path,
    max_dimension: u32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let image_path = image_path.to_path_buf();
    let output_path = output_path.to_path_buf();
    let start_time = Instant::now();

    let result = web::block(move || {
        let is_svg = image_path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("svg"))
            .unwrap_or(false);

        let mut img = if is_svg {
            let svg_data = std::fs::read(&image_path)?;
            let opt = resvg::usvg::Options::default();
            let tree = resvg::usvg::Tree::from_data(&svg_data, &opt)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
            let size = tree.size().to_int_size();
            let mut pixmap = resvg::tiny_skia::Pixmap::new(size.width(), size.height())
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "SVG has zero size"))?;
            resvg::render(&tree, resvg::tiny_skia::Transform::default(), &mut pixmap.as_mut());
            let rgba = image::RgbaImage::from_raw(size.width(), size.height(), pixmap.take())
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "SVG pixel buffer size mismatch"))?;
            image::DynamicImage::ImageRgba8(rgba)
        } else {
            image::open(&image_path)?
        };

        // EXIF Rotation
        let file = std::fs::File::open(&image_path)?;
        let mut bufreader = std::io::BufReader::new(&file);
        let exifreader = kamadak_exif::Reader::new();

        if let Ok(exif) = exifreader.read_from_container(&mut bufreader) {
            if let Some(field) = exif.get_field(kamadak_exif::Tag::Orientation, kamadak_exif::In::PRIMARY) {
                if let kamadak_exif::Value::Short(ref v) = field.value {
                    if let Some(&orientation) = v.first() {
                        match orientation {
                            1 => {}, // Normal
                            2 => img = img.fliph(),
                            3 => img = img.rotate180(),
                            4 => img = img.flipv(),
                            5 => { img = img.rotate90(); img = img.fliph(); },
                            6 => img = img.rotate90(),
                            7 => { img = img.rotate270(); img = img.fliph(); },
                            8 => img = img.rotate270(),
                            _ => {},
                        }
                    }
                }
            }
        }

        let (width, height) = img.dimensions();

        // Calculate aspect-preserving dimensions
        let (thumb_w, thumb_h) = if width > height {
            (max_dimension, (height * max_dimension) / width)
        } else {
            ((width * max_dimension) / height, max_dimension)
        };

        let thumbnail = image::imageops::resize(
            &img, thumb_w, thumb_h, image::imageops::FilterType::Triangle
        );

        let mut output = std::fs::File::create(&output_path)?;
        thumbnail.write_to(&mut output, image::ImageOutputFormat::Jpeg(85))?;
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    }).await?;

    let duration = start_time.elapsed();
    THUMBNAIL_DURATION.observe(duration.as_secs_f64());

    match result {
        Ok(()) => {
            THUMBNAIL_SUCCESS_TOTAL.inc();
            Ok(())
        }
        Err(e) => {
            THUMBNAIL_FAILURES_TOTAL.inc();
            Err(e)
        }
    }
}

/// Generate a thumbnail for a video file using ffmpeg
///
/// Extracts a frame from 1 second into the video (or first frame if shorter)
/// and saves it as a JPEG thumbnail.
///
/// # Arguments
/// * `video_path` - Path to the source video file
/// * `output_path` - Path where the thumbnail should be saved
///
/// # Returns
/// * `Ok(())` if thumbnail generation succeeded
/// * `Err(...)` if thumbnail generation failed
pub async fn generate_thumbnail_for_video(
    video_path: &std::path::Path,
    output_path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let video_path = video_path.to_path_buf();
    let output_path = output_path.to_path_buf();
    let start_time = Instant::now();

    let result = tokio::process::Command::new("ffmpeg")
        .args(&[
            "-i", video_path.to_str().ok_or("Invalid video path")?,
            "-ss", "00:00:01",           // Seek to 1 second
            "-vframes", "1",              // Extract 1 frame
            "-vf", "scale=500:-1",        // Scale to 500px width, preserve aspect ratio
            "-q:v", "2",                  // High quality JPEG
            "-y",                         // Overwrite output
            output_path.to_str().ok_or("Invalid output path")?
        ])
        .output()
        .await;

    let duration = start_time.elapsed();
    THUMBNAIL_DURATION.observe(duration.as_secs_f64());

    match result {
        Ok(output) if output.status.success() => {
            THUMBNAIL_SUCCESS_TOTAL.inc();
            info!("Generated video thumbnail: {:?}", output_path);
            Ok(())
        }
        Ok(output) => {
            THUMBNAIL_FAILURES_TOTAL.inc();
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("ffmpeg failed for {:?}: {}", video_path, stderr);
            Err(format!("ffmpeg failed: {}", stderr).into())
        }
        Err(e) => {
            THUMBNAIL_FAILURES_TOTAL.inc();
            error!("Failed to run ffmpeg for {:?}: {}", video_path, e);
            Err(format!("Failed to run ffmpeg: {}", e).into())
        }
    }
}

#[derive(Deserialize, Debug, ToSchema, IntoParams)]
#[schema(example = json!({
    "page": 1,
    "limit": 50,
    "media_type": "camera",
    "starred_only": false,
    "label_id": 1,
    "start_date": "2024-01-01",
    "end_date": "2024-12-31",
    "location_lat": 51.5074,
    "location_lon": -0.1278,
    "location_radius_km": 10.0
}))]
pub struct PaginationQuery {
    #[serde(default = "default_page")]
    page: usize,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default = "default_type")]
    media_type: String,
    #[serde(default)]
    starred_only: bool,
    /// Optional label ID filter
    #[serde(default)]
    pub label_id: Option<i32>,
    /// Optional start date filter (YYYY-MM-DD format)
    #[serde(default)]
    start_date: Option<String>,
    /// Optional end date filter (YYYY-MM-DD format, inclusive)
    #[serde(default)]
    end_date: Option<String>,
    /// Optional latitude for location filtering
    #[serde(default)]
    pub location_lat: Option<f64>,
    /// Optional longitude for location filtering
    #[serde(default)]
    pub location_lon: Option<f64>,
    /// Optional search radius in kilometers (default: 10)
    #[serde(default)]
    pub location_radius_km: Option<f64>,
    /// Sort order: "date" (default) or "size"
    #[serde(default)]
    pub sort_by: Option<String>,
}

fn default_page() -> usize {
    1
}

fn default_limit() -> usize {
    50 // A reasonable default for a thumbnail grid
}

fn default_type() -> String {
    "all".to_string()
}

#[derive(Serialize, Deserialize, ToSchema)]
#[schema(example = json!({
    "hash": "somehash",
    "name": "IMG_20231222_101010.jpg",
    "created_at": "2025-01-01T12:00:00Z",
    "place": "Paris, France",
    "device_id": "my_device",
    "starred": false,
    "distance_km": 5.2,
    "media_type": "image"
}))]
pub struct ThumbnailItem {
    pub hash: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub place: Option<String>,
    pub device_id: Option<String>,
    pub starred: bool,
    pub distance_km: Option<f32>,
    pub media_type: Option<String>,
    pub thumbnail_url: String,
    pub file_size_bytes: Option<i64>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[schema(example = json!({
    "thumbnails": [],
    "total": 0,
    "page": 1,
    "limit": 50
}))]
pub struct ThumbnailsResponse {
    pub thumbnails: Vec<ThumbnailItem>,
    pub total: usize,
    pub page: usize,
    pub limit: usize,
}

async fn list_media_thumbnails(
    req: HttpRequest,
    query: web::Query<PaginationQuery>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
    req_type: &str, // "images" or "videos"
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match
        utils::authenticate_request(&req, &format!("list_{}_thumbnails", req_type), config.get_api_key())
    {
        Ok(claims) => claims,
        Err(response) => {
            return Ok(response);
        }
    };

    // For admin users, show all media across all devices; for regular users, filter by user_id
    let device_id_filter: Option<String> = None; // Access control is now via user_id in the query

    let page = query.page.max(1);
    let limit = query.limit;
    let offset = (page - 1) * limit;
    let media_type = &query.media_type;
    info!(
        "Listing thumbnails for role: {}, device_id_filter: {:?}, page: {}, limit: {}, type: {}, category: {}",
        claims.role,
        device_id_filter,
        page,
        limit,
        media_type,
        req_type
    );

    // Non-admin users get user_id-based access control
    let apply_user_id_filter = claims.role != "admin";

    let total = utils::total_thumbnails(
        &claims.user_id,
        device_id_filter.as_deref(),
        req_type,
        media_type,
        query.starred_only,
        query.start_date.as_deref(),
        query.end_date.as_deref(),
        query.location_lat,
        query.location_lon,
        query.location_radius_km,
        query.label_id,
        apply_user_id_filter,
        &pool
    ).await;

    let thumbnails = utils
        ::list_thumbnails(
            &claims.user_id,
            device_id_filter.as_deref(),
            req_type,
            media_type,
            offset,
            limit,
            query.starred_only,
            query.start_date.as_deref(),
            query.end_date.as_deref(),
            query.location_lat,
            query.location_lon,
            query.location_radius_km,
            query.label_id,
            apply_user_id_filter,
            query.sort_by.as_deref(),
            &pool
        ).await
        .map_err(|e| {
            error!("Failed to list thumbnails: {}", e);
            actix_web::error::ErrorInternalServerError("Failed to list thumbnails")
        })?;
        
    let response_data = ThumbnailsResponse {
        thumbnails,
        total: total as usize,
        page,
        limit,
    };

    Ok(HttpResponse::Ok().json(response_data))
}

#[utoipa::path(
    get,
    path = "/image_thumbnails",
    params(PaginationQuery),
    responses(
        (status = 200, description = "List of thumbnails", body = ThumbnailsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    )
)]
#[get("/image_thumbnails")]
pub async fn list_image_thumbnails(
    req: HttpRequest,
    query: web::Query<PaginationQuery>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>
) -> Result<HttpResponse, actix_web::Error> {
    list_media_thumbnails(req, query, pool, config, "images").await
}

#[utoipa::path(
    get,
    path = "/video_thumbnails",
    params(PaginationQuery),
    responses(
        (status = 200, description = "List of thumbnails", body = ThumbnailsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    )
)]
#[get("/video_thumbnails")]
pub async fn list_video_thumbnails(
    req: HttpRequest,
    query: web::Query<PaginationQuery>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>
) -> Result<HttpResponse, actix_web::Error> {
    list_media_thumbnails(req, query, pool, config, "videos").await
}

#[utoipa::path(
    get,
    path = "/media_thumbnails",
    params(PaginationQuery),
    responses(
        (status = 200, description = "List of all media thumbnails (images and videos)", body = ThumbnailsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    )
)]
#[get("/media_thumbnails")]
pub async fn list_all_media_thumbnails(
    req: HttpRequest,
    query: web::Query<PaginationQuery>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>
) -> Result<HttpResponse, actix_web::Error> {
    list_media_thumbnails(req, query, pool, config, "all").await
}

#[utoipa::path(
    get,
    path = "/thumbnail/{media_hash}",
    responses(
        (status = 200, description = "Thumbnail found"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Thumbnail not found"),
        (status = 500, description = "Internal server error")
    )
)]
#[get("/thumbnail/{media_hash}")]
pub async fn get_thumbnail(
    req: HttpRequest,
    path: web::Path<String>,
    config: web::Data<Config>
) -> Result<HttpResponse, actix_web::Error> {
    if
        let Err(response) = utils::authenticate_request(
            &req,
            "get_thumbnail",
            config.get_api_key()
        )
    {
        return Ok(response);
    }

    let media_hash = path.into_inner();
    let thumb_filename = format!("{}.thumb.jpg", &media_hash);

    // First try to find thumbnail in images directory with subdirectory structure
    let image_sub_dir_path = utils::get_subdirectory_path(config.get_images_dir(), &media_hash);
    let image_thumb_path = image_sub_dir_path.join(&thumb_filename);
    match async_fs::read(&image_thumb_path).await {
        Ok(data) => {
            return Ok(HttpResponse::Ok()
                .content_type("image/jpeg")
                .insert_header(("Cache-Control", "public, max-age=31536000, immutable"))
                .body(data));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Continue to check video directory
        }
        Err(e) => {
            error!(
                "Failed to read image thumbnail file. Hash: '{}', Path: {:?}, Error: {}",
                &media_hash,
                &image_thumb_path,
                e
            );
            return Err(
                actix_web::error::ErrorInternalServerError("Could not read thumbnail file.")
            );
        }
    }

    // Try to find thumbnail in videos directory with subdirectory structure
    let video_sub_dir_path = utils::get_subdirectory_path(config.get_videos_dir(), &media_hash);
    let video_thumb_path = video_sub_dir_path.join(&thumb_filename);
    match async_fs::read(&video_thumb_path).await {
        Ok(data) => Ok(HttpResponse::Ok()
            .content_type("image/jpeg")
            .insert_header(("Cache-Control", "public, max-age=31536000, immutable"))
            .body(data)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            warn!(
                "Thumbnail not found for hash: '{}'. Checked paths: {:?}, {:?})",
                &media_hash,
                &image_thumb_path,
                &video_thumb_path
            );
            Ok(
                HttpResponse::NotFound().json(
                    serde_json::json!({"status": "error", "message": "Thumbnail not found."})
                )
            )
        }
        Err(e) => {
            error!(
                "Failed to read video thumbnail file. Hash: '{}', Path: {:?}, Error: {}",
                &media_hash,
                &video_thumb_path,
                e
            );
            Err(actix_web::error::ErrorInternalServerError("Could not read thumbnail file."))
        }
    }
}