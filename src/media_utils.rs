use actix_web::web;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use futures::TryStreamExt;
use log::{error, info, warn};
use std::path::PathBuf;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::constants::media;
use crate::db::MainDbPool;
use crate::query_builder::MediaQueryBuilder;
use crate::services::thumbnail::ThumbnailItem;

// ---- EXIF Orientation Helpers ------------------------------------------------

/// Apply an EXIF orientation value to a `DynamicImage`, returning the correctly rotated image.
/// Orientation 1 (normal) and any unknown value are returned unchanged.
pub fn apply_orientation_to_image(
    img: image::DynamicImage,
    orientation: u16,
) -> image::DynamicImage {
    match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img, // 1 = normal; unknown values are left unchanged
    }
}

/// Read the EXIF orientation tag from in-memory image bytes.
/// Returns `None` if no orientation tag is present or the bytes aren't valid EXIF.
pub fn read_exif_orientation_from_bytes(data: &[u8]) -> Option<u16> {
    let cursor = std::io::Cursor::new(data);
    let mut bufreader = std::io::BufReader::new(cursor);
    kamadak_exif::Reader::new()
        .read_from_container(&mut bufreader)
        .ok()
        .and_then(|exif| {
            exif.get_field(kamadak_exif::Tag::Orientation, kamadak_exif::In::PRIMARY)
                .and_then(|f| {
                    if let kamadak_exif::Value::Short(ref v) = f.value {
                        v.first().copied()
                    } else {
                        None
                    }
                })
        })
}

/// Read the EXIF orientation tag by opening the file at `path`.
/// Returns `None` if the file can't be opened, has no EXIF, or has no orientation tag.
pub fn read_exif_orientation_from_path(path: &std::path::Path) -> Option<u16> {
    let file = std::fs::File::open(path).ok()?;
    let mut bufreader = std::io::BufReader::new(&file);
    kamadak_exif::Reader::new()
        .read_from_container(&mut bufreader)
        .ok()
        .and_then(|exif| {
            exif.get_field(kamadak_exif::Tag::Orientation, kamadak_exif::In::PRIMARY)
                .and_then(|f| {
                    if let kamadak_exif::Value::Short(ref v) = f.value {
                        v.first().copied()
                    } else {
                        None
                    }
                })
        })
}

// ---- JPEG EXIF Orientation Injection ----------------------------------------

/// Inject a minimal EXIF APP1 block carrying only the Orientation tag into a
/// JPEG byte stream.  Returns the bytes unchanged if:
///   - the input is not a JPEG (no SOI marker), or
///   - an APP1 EXIF block is already present (shouldn't happen under the
///     `exif IS NULL` DB guard, but safe to handle).
///
/// The injected block is 36 bytes inserted right after the 2-byte SOI marker.
/// No decode/re-encode — pixel data is never touched.
pub fn inject_exif_orientation(jpeg_bytes: &[u8], orientation: u16) -> Vec<u8> {
    // Must start with JPEG SOI (FF D8)
    if jpeg_bytes.len() < 4 || jpeg_bytes[0] != 0xFF || jpeg_bytes[1] != 0xD8 {
        return jpeg_bytes.to_vec();
    }

    // Scan segments after SOI; bail out if an APP1 EXIF block already exists
    let mut pos = 2usize;
    while pos + 4 <= jpeg_bytes.len() {
        if jpeg_bytes[pos] != 0xFF {
            break;
        }
        let marker = jpeg_bytes[pos + 1];
        if marker == 0xE1
            && pos + 10 <= jpeg_bytes.len()
            && &jpeg_bytes[pos + 4..pos + 10] == b"Exif\0\0"
        {
            return jpeg_bytes.to_vec();
        }
        let seg_len = u16::from_be_bytes([jpeg_bytes[pos + 2], jpeg_bytes[pos + 3]]) as usize;
        if seg_len < 2 {
            break;
        }
        pos += 2 + seg_len;
    }

    // Build a 36-byte minimal EXIF APP1 block (little-endian TIFF, 1 IFD entry)
    let mut app1 = [0u8; 36];
    app1[0..2].copy_from_slice(&[0xFF, 0xE1]);         // APP1 marker
    app1[2..4].copy_from_slice(&[0x00, 0x22]);         // length = 34
    app1[4..10].copy_from_slice(b"Exif\0\0");          // EXIF header
    app1[10..12].copy_from_slice(&[0x49, 0x49]);       // "II" little-endian
    app1[12..14].copy_from_slice(&[0x2A, 0x00]);       // TIFF magic
    app1[14..18].copy_from_slice(&[0x08, 0x00, 0x00, 0x00]); // IFD0 at offset 8
    app1[18..20].copy_from_slice(&[0x01, 0x00]);       // 1 IFD entry
    app1[20..22].copy_from_slice(&[0x12, 0x01]);       // tag 0x0112 = Orientation
    app1[22..24].copy_from_slice(&[0x03, 0x00]);       // type SHORT
    app1[24..28].copy_from_slice(&[0x01, 0x00, 0x00, 0x00]); // count 1
    app1[28] = (orientation & 0xFF) as u8;             // value low byte
    app1[29] = (orientation >> 8) as u8;               // value high byte
    // bytes 30-35 stay zero (SHORT padding + next-IFD offset)

    let mut out = Vec::with_capacity(jpeg_bytes.len() + 36);
    out.extend_from_slice(&jpeg_bytes[..2]); // SOI
    out.extend_from_slice(&app1);
    out.extend_from_slice(&jpeg_bytes[2..]);
    out
}

/// Rotate a PNG's pixel data according to an EXIF orientation value and
/// re-encode as PNG (lossless).  Returns `None` if the bytes cannot be decoded.
pub fn rotate_png_bytes(png_bytes: &[u8], orientation: u16) -> Option<Vec<u8>> {
    let img = image::load_from_memory(png_bytes).ok()?;
    let rotated = apply_orientation_to_image(img, orientation);
    let mut buf = std::io::Cursor::new(Vec::new());
    rotated.write_to(&mut buf, image::ImageOutputFormat::Png).ok()?;
    Some(buf.into_inner())
}

// ---- Path / Type Helpers ----------------------------------------------------

/// Compute the BLAKE3 hash of a file, returning the hex string.
pub async fn hash_file_blake3(path: &std::path::Path) -> Result<String, std::io::Error> {
    let mut file = fs::File::open(path).await?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; 8192];
    loop {
        match file.read(&mut buffer).await {
            Ok(0) => break,
            Ok(n) => { hasher.update(&buffer[..n]); }
            Err(e) => return Err(e),
        }
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Stream a multipart field to a temp file while computing its BLAKE3 hash.
/// Returns `(temp_path, blake3_hex_hash)`.
pub async fn streaming_hash_to_temp(
    field: &mut actix_multipart::Field,
    temp_dir: &std::path::Path,
) -> Result<(PathBuf, String), actix_web::Error> {
    let effective_temp_dir = if tokio::fs::create_dir_all(temp_dir).await.is_ok() {
        temp_dir.to_path_buf()
    } else {
        let fallback = std::env::temp_dir().join("reminisce_uploads");
        let _ = tokio::fs::create_dir_all(&fallback).await;
        fallback
    };

    let temp_filename = format!("{}.tmp", uuid::Uuid::new_v4());
    let mut temp_path = effective_temp_dir.join(&temp_filename);

    let mut f = match tokio::fs::File::create(&temp_path).await {
        Ok(file) => file,
        Err(e) => {
            log::warn!("Failed to create temp file at {:?}: {}. Falling back to system temp dir.", temp_path, e);
            let fallback_dir = std::env::temp_dir().join("reminisce_uploads");
            let _ = tokio::fs::create_dir_all(&fallback_dir).await;
            temp_path = fallback_dir.join(&temp_filename);
            tokio::fs::File::create(&temp_path).await.map_err(|err| {
                log::error!("Fallback temp file creation failed at {:?}: {}", temp_path, err);
                actix_web::error::ErrorInternalServerError("Failed to create temp file")
            })?
        }
    };

    let mut hasher = blake3::Hasher::new();
    while let Ok(Some(chunk)) = field.try_next().await {
        hasher.update(&chunk);
        f.write_all(&chunk).await
            .map_err(|e| {
                log::error!("Failed to write temp file {:?}: {}", temp_path, e);
                actix_web::error::ErrorInternalServerError("Failed to write temp file")
            })?;
    }
    Ok((temp_path, hasher.finalize().to_hex().to_string()))
}

/// Drain a multipart text field into a `String`.
pub async fn read_field_string(field: &mut actix_multipart::Field) -> String {
    let mut bytes = Vec::new();
    while let Ok(Some(chunk)) = field.try_next().await {
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Decode `image_data`, apply the given EXIF `orientation`, and re-encode as JPEG (quality 90).
pub fn orient_image_to_jpeg(image_data: &[u8], orientation: u16) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory(image_data)
        .map_err(|e| format!("Failed to decode image: {}", e))?;
    let oriented = apply_orientation_to_image(img, orientation);
    let mut output = std::io::Cursor::new(Vec::new());
    oriented
        .write_to(&mut output, image::ImageOutputFormat::Jpeg(90))
        .map_err(|e| format!("Failed to encode oriented image: {}", e))?;
    Ok(output.into_inner())
}

/// Generates a two-character subdirectory path from the first two characters of a hash.
pub fn get_subdirectory_path(base_dir: &str, hash: &str) -> PathBuf {
    if hash.len() < 2 {
        return PathBuf::from(base_dir);
    }
    PathBuf::from(base_dir).join(&hash[..2])
}

/// True if `s` is a 64-character lowercase hex string (the BLAKE3 content-hash format).
/// Content hashes are interpolated into filesystem paths, so anything else is rejected
/// to prevent path traversal.
pub fn is_valid_content_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// True if `s` is a safe media-file extension: 1–10 alphanumeric characters.
/// `ext` is interpolated into the served filename, so any path separator, dot, or
/// control character is rejected to prevent path traversal.
pub fn is_valid_media_ext(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 10
        && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Resolve the on-disk path for a content-addressed media file, canonicalizing the
/// base directory and verifying the resolved file stays inside it. Prevents LFI /
/// path traversal via user-supplied hash or ext.
pub fn safe_resolve_content_path(
    base_dir: &str,
    hash: &str,
    ext: &str,
) -> Result<PathBuf, String> {
    if !is_valid_content_hash(hash) {
        return Err(format!("Invalid content hash: {}", hash));
    }
    if !is_valid_media_ext(ext) {
        return Err(format!("Invalid media extension: {}", ext));
    }

    let base = std::fs::canonicalize(base_dir)
        .map_err(|e| format!("Cannot canonicalize base dir {}: {}", base_dir, e))?;
    let resolved = base.join(&hash[..2]).join(format!("{}.{}", hash, ext));
    let canonical = std::fs::canonicalize(&resolved)
        .map_err(|e| format!("Cannot canonicalize {}: {}", resolved.display(), e))?;
    if !canonical.starts_with(&base) {
        return Err(format!("Resolved path escapes media directory: {}", canonical.display()));
    }
    Ok(canonical)
}

pub fn determine_image_type(image_name: &str) -> String {
    let lower_name = image_name.to_lowercase();
    if lower_name.contains("dcim/camera") {
        media::TYPE_CAMERA.to_string()
    } else if lower_name.contains("whatsapp") {
        media::TYPE_WHATSAPP.to_string()
    } else if lower_name.contains("screenshot") {
        media::TYPE_SCREENSHOT.to_string()
    } else {
        media::TYPE_OTHER.to_string()
    }
}

pub fn determine_video_type(video_name: &str) -> String {
    let lower_name = video_name.to_lowercase();
    if lower_name.contains("dcim/camera") || lower_name.contains("dji") {
        media::TYPE_CAMERA.to_string()
    } else if lower_name.contains("whatsapp") {
        media::TYPE_WHATSAPP.to_string()
    } else if lower_name.contains("screen") {
        media::TYPE_SCREEN_RECORDING.to_string()
    } else {
        media::TYPE_OTHER.to_string()
    }
}

// ---- Existence Check --------------------------------------------------------

#[derive(serde::Serialize)]
pub struct ExistenceCheckResult {
    pub exists_for_user: bool,
    pub exists_verified: bool,
}

pub async fn check_if_exists(
    hash: &str,
    user_id: &uuid::Uuid,
    table: &str,
    pool: web::Data<MainDbPool>,
) -> Result<ExistenceCheckResult, tokio_postgres::Error> {
    let client = pool.0.get().await.expect("Failed to get database client");

    let query_string = match table {
        "images" => "
            SELECT
                EXISTS(SELECT 1 FROM images WHERE user_id = $1 AND hash = $2 AND deleted_at IS NULL) as exists_for_user,
                EXISTS(SELECT 1 FROM images WHERE user_id = $1 AND hash = $2 AND verification_status = 1 AND deleted_at IS NULL) as exists_verified
        ",
        "videos" => "
            SELECT
                EXISTS(SELECT 1 FROM videos WHERE user_id = $1 AND hash = $2 AND deleted_at IS NULL) as exists_for_user,
                EXISTS(SELECT 1 FROM videos WHERE user_id = $1 AND hash = $2 AND verification_status = 1 AND deleted_at IS NULL) as exists_verified
        ",
        _ => {
            warn!("Invalid table name provided to check_if_exists: {}", table);
            return Err(tokio_postgres::Error::__private_api_timeout());
        }
    };

    let row = client.query_one(query_string, &[user_id, &hash]).await?;
    Ok(ExistenceCheckResult {
        exists_for_user: row.get(0),
        exists_verified: row.get(1),
    })
}

// ---- Thumbnail Listing ------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub async fn list_thumbnails(
    user_id: &str,
    device_id: Option<&str>,
    table: &str,
    media_type: &str,
    offset: usize,
    limit: usize,
    starred_only: bool,
    start_date: Option<&str>,
    end_date: Option<&str>,
    location_lat: Option<f64>,
    location_lon: Option<f64>,
    location_radius_km: Option<f64>,
    label_id: Option<i32>,
    apply_user_id_filter: bool,
    no_exif: bool,
    sort_by: Option<&str>,
    sort_order: Option<&str>,
    pool: &web::Data<MainDbPool>,
) -> Result<Vec<ThumbnailItem>, Box<dyn std::error::Error>> {
    let client = pool.0.get().await.map_err(|e| {
        error!("Failed to get database client for list_thumbnails: {}", e);
        Box::new(e) as Box<dyn std::error::Error>
    })?;

    let user_uuid = uuid::Uuid::parse_str(user_id).map_err(|e| {
        error!("Failed to parse user_id as UUID: {}", e);
        Box::new(e) as Box<dyn std::error::Error>
    })?;

    let apply_filters = |builder: &mut MediaQueryBuilder| {
        builder.with_user_id();
        if apply_user_id_filter {
            builder.with_user_id_filter();
        }
        if device_id.is_some() {
            builder.with_device_id();
        }
        // The `type` column is unpopulated, so media type must be scoped by table:
        // `all` keeps both tables, `image`/`video` keep only their own and zero out
        // the other (postgres `type` column cannot be trusted for this filter).
        let wrong_table = (media_type == media::TYPE_IMAGE && !builder.is_images_table())
            || (media_type == media::TYPE_VIDEO && !builder.is_videos_table());
        if wrong_table {
            builder.add_custom_condition("1 = 0".to_string());
        }
        if starred_only {
            builder.with_starred_only();
        }
        if label_id.is_some() {
            builder.with_label_id();
        }
        if start_date.is_some() {
            builder.with_start_date();
        }
        if end_date.is_some() {
            builder.with_end_date();
        }
        if no_exif && builder.is_images_table() {
            builder.with_no_exif();
        }
    };

    let query_string;
    let has_location_filter = location_lat.is_some() && location_lon.is_some();
    let limit_param;
    let offset_param;
    let mut lon_param_idx = None;
    let mut lat_param_idx = None;

    if table == "all" {
        let mut img_builder = MediaQueryBuilder::new("images");
        apply_filters(&mut img_builder);

        if has_location_filter {
            let radius_km = location_radius_km.unwrap_or(10.0);
            let radius_meters = radius_km * 1000.0;
            let lon_param = img_builder.param_count() + 1;
            let lat_param = img_builder.param_count() + 2;
            img_builder.add_custom_condition("t.location IS NOT NULL".to_string());
            img_builder.add_custom_condition(format!(
                "ST_DWithin(t.location, ST_MakePoint(${}, ${})::geography, {})",
                lon_param, lat_param, radius_meters
            ));
            lon_param_idx = Some(lon_param);
            lat_param_idx = Some(lat_param);
        }

        let mut vid_builder = MediaQueryBuilder::new("videos");
        apply_filters(&mut vid_builder);
        if has_location_filter {
            vid_builder.add_custom_condition("1 = 0".to_string());
        }

        let max_param =
            img_builder.param_count() + (if has_location_filter { 2 } else { 0 });
        limit_param = max_param + 1;
        offset_param = max_param + 2;

        let img_body = img_builder.build_select_body(lon_param_idx, lat_param_idx);
        let vid_body = vid_builder.build_select_body(None, None);

        let dir = if sort_order == Some("asc") { "ASC" } else { "DESC" };
        let order_clause = if sort_by == Some("size") {
            format!("ORDER BY file_size_bytes {} NULLS LAST, hash {}", dir, dir)
        } else if sort_by == Some("quality") {
            format!("ORDER BY aesthetic_score {} NULLS LAST, hash {}", dir, dir)
        } else {
            format!("ORDER BY created_at {}, hash {}", dir, dir)
        };

        query_string = format!(
            "SELECT * FROM (\
                SELECT DISTINCT ON (hash) hash, name, created_at, place, deviceid, starred, \
                    distance_km, media_type, file_size_bytes, aesthetic_score \
                FROM ({} UNION ALL {}) combined \
                ORDER BY hash, aesthetic_score DESC NULLS LAST\
            ) deduped {} LIMIT ${} OFFSET ${}",
            img_body, vid_body, order_clause, limit_param, offset_param
        );
    } else {
        let mut builder = MediaQueryBuilder::new(table);
        apply_filters(&mut builder);

        if has_location_filter && table == "images" {
            let radius_km = location_radius_km.unwrap_or(10.0);
            let radius_meters = radius_km * 1000.0;
            let lon_param = builder.param_count() + 1;
            let lat_param = builder.param_count() + 2;
            builder.add_custom_condition("t.location IS NOT NULL".to_string());
            builder.add_custom_condition(format!(
                "ST_DWithin(t.location, ST_MakePoint(${}, ${})::geography, {})",
                lon_param, lat_param, radius_meters
            ));
            lon_param_idx = Some(lon_param);
            lat_param_idx = Some(lat_param);
        }

        limit_param = builder.param_count()
            + 1
            + (if has_location_filter && table == "images" { 2 } else { 0 });
        offset_param = builder.param_count()
            + 2
            + (if has_location_filter && table == "images" { 2 } else { 0 });

        query_string = builder.build_select_query(
            limit_param,
            offset_param,
            lon_param_idx,
            lat_param_idx,
            sort_by,
            sort_order,
        );
    }

    let limit_i64 = limit as i64;
    let offset_i64 = offset as i64;

    use chrono::TimeZone;
    let start_datetime: Option<DateTime<Utc>> = start_date.and_then(|d| {
        NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .ok()
            .and_then(|nd| nd.and_hms_opt(0, 0, 0))
            .map(|ndt| Utc.from_utc_datetime(&ndt))
    });
    let end_datetime: Option<DateTime<Utc>> = end_date.and_then(|d| {
        NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .ok()
            .and_then(|nd| nd.and_hms_opt(23, 59, 59))
            .and_then(|ndt| ndt.checked_add_signed(chrono::Duration::seconds(1)))
            .map(|ndt| Utc.from_utc_datetime(&ndt))
    });

    let device_id_value;
    let label_id_value;
    let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![&user_uuid];

    if let Some(dev_id) = device_id {
        device_id_value = dev_id;
        params.push(&device_id_value as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    if let Some(lbl_id) = label_id {
        label_id_value = lbl_id;
        params.push(&label_id_value as &(dyn tokio_postgres::types::ToSql + Sync));
    }
    if let Some(ref sd) = start_datetime {
        params.push(sd as &(dyn tokio_postgres::types::ToSql + Sync));
    }
    if let Some(ref ed) = end_datetime {
        params.push(ed as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    let lat_value;
    let lon_value;
    if lon_param_idx.is_some() {
        lat_value = location_lat.expect("lon_param_idx is only set when location_lat is Some");
        lon_value = location_lon.expect("lon_param_idx is only set when location_lon is Some");
        params.push(&lon_value as &(dyn tokio_postgres::types::ToSql + Sync));
        params.push(&lat_value as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    params.push(&limit_i64 as &(dyn tokio_postgres::types::ToSql + Sync));
    params.push(&offset_i64 as &(dyn tokio_postgres::types::ToSql + Sync));

    let rows = client.query(&query_string, &params).await?;

    let thumbnails = rows
        .into_iter()
        .map(|row| {
            let distance_km: Option<f64> = row.get("distance_km");
            let media_type_val: Option<String> = row.try_get("media_type").ok();
            let final_media_type = media_type_val.or_else(|| {
                if table == "images" {
                    Some("image".to_string())
                } else if table == "videos" {
                    Some("video".to_string())
                } else {
                    None
                }
            });
            let hash: String = row.get("hash");
            ThumbnailItem {
                hash: hash.clone(),
                name: row.get("name"),
                created_at: row.get("created_at"),
                place: row.get("place"),
                device_id: row.get("deviceid"),
                starred: row.get("starred"),
                distance_km: distance_km.map(|d| d as f32),
                media_type: final_media_type,
                thumbnail_url: format!("/api/thumbnail/{}", hash),
                file_size_bytes: row.try_get("file_size_bytes").unwrap_or(None),
                aesthetic_score: row.try_get("aesthetic_score").unwrap_or(None),
            }
        })
        .collect();

    Ok(thumbnails)
}

#[allow(clippy::too_many_arguments)]
pub async fn total_thumbnails(
    user_id: &str,
    device_id: Option<&str>,
    table: &str,
    media_type: &str,
    starred_only: bool,
    start_date: Option<&str>,
    end_date: Option<&str>,
    location_lat: Option<f64>,
    location_lon: Option<f64>,
    location_radius_km: Option<f64>,
    label_id: Option<i32>,
    apply_user_id_filter: bool,
    no_exif: bool,
    pool: &web::Data<MainDbPool>,
) -> i64 {
    let client = match pool.0.get().await {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to get database client for total_thumbnails: {}", e);
            return 0;
        }
    };

    let user_uuid = match uuid::Uuid::parse_str(user_id) {
        Ok(u) => u,
        Err(e) => {
            error!("Failed to parse user_id as UUID in total_thumbnails: {}", e);
            return 0;
        }
    };

    let apply_filters = |builder: &mut MediaQueryBuilder| {
        builder.with_has_thumbnail();
        builder.with_user_id();
        if apply_user_id_filter {
            builder.with_user_id_filter();
        }
        if device_id.is_some() {
            builder.with_device_id();
        }
        // The `type` column is unpopulated, so media type must be scoped by table:
        // `all` keeps both tables, `image`/`video` keep only their own and zero out
        // the other (postgres `type` column cannot be trusted for this filter).
        let wrong_table = (media_type == media::TYPE_IMAGE && !builder.is_images_table())
            || (media_type == media::TYPE_VIDEO && !builder.is_videos_table());
        if wrong_table {
            builder.add_custom_condition("1 = 0".to_string());
        }
        if starred_only {
            builder.with_starred_only();
        }
        if label_id.is_some() {
            builder.with_label_id();
        }
        if start_date.is_some() {
            builder.with_start_date();
        }
        if end_date.is_some() {
            builder.with_end_date();
        }
        if no_exif && builder.is_images_table() {
            builder.with_no_exif();
        }
    };

    let query_string;
    let has_location_filter = location_lat.is_some() && location_lon.is_some();
    let mut lon_param_idx = None;

    if table == "all" {
        let mut img_builder = MediaQueryBuilder::new("images");
        apply_filters(&mut img_builder);

        if has_location_filter {
            let radius_km = location_radius_km.unwrap_or(10.0);
            let radius_meters = radius_km * 1000.0;
            let lon_param = img_builder.param_count() + 1;
            let lat_param = img_builder.param_count() + 2;
            img_builder.add_custom_condition("t.location IS NOT NULL".to_string());
            img_builder.add_custom_condition(format!(
                "ST_DWithin(t.location, ST_MakePoint(${}, ${})::geography, {})",
                lon_param, lat_param, radius_meters
            ));
            lon_param_idx = Some(lon_param);
        }

        let mut vid_builder = MediaQueryBuilder::new("videos");
        apply_filters(&mut vid_builder);
        if has_location_filter {
            vid_builder.add_custom_condition("1 = 0".to_string());
        }

        let img_count_query = img_builder.build_count_query(starred_only);
        let vid_count_query = vid_builder.build_count_query(starred_only);
        query_string = format!("SELECT ({}) + ({})", img_count_query, vid_count_query);
    } else {
        let mut builder = MediaQueryBuilder::new(table);
        apply_filters(&mut builder);

        if has_location_filter && table == "images" {
            let radius_km = location_radius_km.unwrap_or(10.0);
            let radius_meters = radius_km * 1000.0;
            let lon_param = builder.param_count() + 1;
            let lat_param = builder.param_count() + 2;
            builder.add_custom_condition("t.location IS NOT NULL".to_string());
            builder.add_custom_condition(format!(
                "ST_DWithin(t.location, ST_MakePoint(${}, ${})::geography, {})",
                lon_param, lat_param, radius_meters
            ));
            lon_param_idx = Some(lon_param);
        }

        query_string = builder.build_count_query(starred_only);
    }

    use chrono::TimeZone;
    let start_datetime: Option<DateTime<Utc>> = start_date.and_then(|d| {
        NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .ok()
            .and_then(|nd| nd.and_hms_opt(0, 0, 0))
            .map(|ndt| Utc.from_utc_datetime(&ndt))
    });
    let end_datetime: Option<DateTime<Utc>> = end_date.and_then(|d| {
        NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .ok()
            .and_then(|nd| nd.and_hms_opt(23, 59, 59))
            .and_then(|ndt| ndt.checked_add_signed(chrono::Duration::seconds(1)))
            .map(|ndt| Utc.from_utc_datetime(&ndt))
    });

    let device_id_value;
    let label_id_value;
    let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![&user_uuid];

    if let Some(dev_id) = device_id {
        device_id_value = dev_id;
        params.push(&device_id_value as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    if let Some(lbl_id) = label_id {
        label_id_value = lbl_id;
        params.push(&label_id_value as &(dyn tokio_postgres::types::ToSql + Sync));
    }
    if let Some(ref sd) = start_datetime {
        params.push(sd as &(dyn tokio_postgres::types::ToSql + Sync));
    }
    if let Some(ref ed) = end_datetime {
        params.push(ed as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    let lat_value;
    let lon_value;
    if lon_param_idx.is_some() {
        lat_value = location_lat.expect("lon_param_idx is only set when location_lat is Some");
        lon_value = location_lon.expect("lon_param_idx is only set when location_lon is Some");
        params.push(&lon_value as &(dyn tokio_postgres::types::ToSql + Sync));
        params.push(&lat_value as &(dyn tokio_postgres::types::ToSql + Sync));
    }

    let row = match client.query_one(&query_string, &params).await {
        Ok(r) => r,
        Err(e) => {
            error!("Failed to get total count: {}", e);
            return 0;
        }
    };

    let total: i64 = row.get(0);
    if let Some(dev_id) = device_id {
        info!("Total thumbnails for device {}: {}", dev_id, total);
    } else {
        info!("Total thumbnails (all devices): {}", total);
    }
    total
}

// ---- Date Parsing -----------------------------------------------------------

fn try_parse_datetime_underscore(name: &str, start_pos: usize) -> Option<DateTime<Utc>> {
    if name.len() < start_pos + 15 {
        return None;
    }
    if name.chars().nth(start_pos + 8) != Some('_') {
        return None;
    }
    let date_part = name.get(start_pos..start_pos + 8)?;
    let time_part = name.get(start_pos + 9..start_pos + 15)?;
    if !date_part.chars().all(|c| c.is_ascii_digit())
        || !time_part.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let datetime_str = format!("{} {}", date_part, time_part);
    NaiveDateTime::parse_from_str(&datetime_str, "%Y%m%d %H%M%S")
        .ok()
        .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc))
}

fn try_parse_whatsapp_format(name: &str, start_pos: usize) -> Option<DateTime<Utc>> {
    let date_end = start_pos + 8;
    if name.len() < date_end + 7 || name.get(date_end..date_end + 3) != Some("-wa") {
        return None;
    }
    let date_part = name.get(start_pos..date_end)?;
    let millis_part = name.get(date_end + 3..date_end + 7)?;
    if !date_part.chars().all(|c| c.is_ascii_digit())
        || !millis_part.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let naive_date = NaiveDate::parse_from_str(date_part, "%Y%m%d").ok()?;
    let millis = millis_part.parse::<u32>().ok()?;
    let actual_millis = millis % 1000;
    Some(DateTime::<Utc>::from_naive_utc_and_offset(
        naive_date.and_hms_milli_opt(0, 0, 0, actual_millis)?,
        Utc,
    ))
}

fn try_parse_date_only(name: &str, start_pos: usize) -> Option<DateTime<Utc>> {
    let date_part = name.get(start_pos..start_pos + 8)?;
    if !date_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    NaiveDate::parse_from_str(date_part, "%Y%m%d")
        .ok()
        .map(|date| {
            DateTime::<Utc>::from_naive_utc_and_offset(
                date.and_hms_opt(0, 0, 0).unwrap(),
                Utc,
            )
        })
}

pub fn parse_date_from_image_name(image_name: &str) -> Option<DateTime<Utc>> {
    let lower_name = image_name.to_lowercase();

    if let Some(pos) = lower_name.find("img_") {
        if let Some(dt) = try_parse_datetime_underscore(&lower_name, pos + 4) {
            return Some(dt);
        }
    }
    if let Some(pos) = lower_name.find("img-") {
        if let Some(dt) = try_parse_whatsapp_format(&lower_name, pos + 4) {
            return Some(dt);
        }
        if let Some(dt) = try_parse_date_only(&lower_name, pos + 4) {
            return Some(dt);
        }
    }

    None
}

pub fn parse_date_from_video_name(video_name: &str) -> Option<DateTime<Utc>> {
    let lower_name = video_name.to_lowercase();

    if let Some(pos) = lower_name.find("dji_") {
        if let Some(dt) = try_parse_datetime_underscore(&lower_name, pos + 4) {
            return Some(dt);
        }
    }
    if let Some(pos) = lower_name.find("sl_mo_vid_") {
        if let Some(dt) = try_parse_datetime_underscore(&lower_name, pos + 10) {
            return Some(dt);
        }
    }
    if let Some(pos) = lower_name.find("vid_") {
        if let Some(dt) = try_parse_datetime_underscore(&lower_name, pos + 4) {
            return Some(dt);
        }
    }
    if let Some(pos) = lower_name.find("vid-") {
        if let Some(dt) = try_parse_whatsapp_format(&lower_name, pos + 4) {
            return Some(dt);
        }
        if let Some(dt) = try_parse_date_only(&lower_name, pos + 4) {
            return Some(dt);
        }
    }

    None
}

// ---- Temp File Cleanup ------------------------------------------------------

pub async fn cleanup_temp_files(
    image_temp_path: Option<PathBuf>,
    thumbnail_temp_path: Option<PathBuf>,
) {
    if let Some(path) = image_temp_path {
        if path.exists() {
            if let Err(e) = fs::remove_file(&path).await {
                warn!("Failed to remove temporary image file {:?}: {}", path, e);
            }
        }
    }
    if let Some(path) = thumbnail_temp_path {
        if path.exists() {
            if let Err(e) = fs::remove_file(&path).await {
                warn!("Failed to remove temporary thumbnail file {:?}: {}", path, e);
            }
        }
    }
}

/// Fire-and-forget version of `cleanup_temp_files` for use in error paths.
pub fn cleanup_temp_files_spawn(
    image_temp_path: Option<PathBuf>,
    thumbnail_temp_path: Option<PathBuf>,
) {
    tokio::spawn(async move {
        cleanup_temp_files(image_temp_path, thumbnail_temp_path).await;
    });
}

/// Extract video keyframe JPEG images at regular intervals using ffmpeg.
/// Returns a list of (timestamp_secs, jpeg_bytes) tuples.
pub async fn extract_video_keyframes(
    video_path: &std::path::Path,
    interval_secs: f32,
    max_keyframes: usize,
) -> Result<Vec<(f32, Vec<u8>)>, String> {
    let video_str = video_path.to_str().ok_or("Invalid video path")?;
    let temp_dir = std::env::temp_dir().join(format!("keyframes_{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .map_err(|e| format!("Failed to create temp keyframes dir: {}", e))?;

    let output_pattern = temp_dir.join("frame_%03d.jpg");
    let output_pattern_str = output_pattern.to_str().ok_or("Invalid output pattern path")?;

    let fps_filter = format!("fps=1/{}", interval_secs);
    let result = tokio::process::Command::new("ffmpeg")
        .args([
            "-i", video_str,
            "-vf", &fps_filter,
            "-vframes", &max_keyframes.to_string(),
            "-q:v", "3",
            "-y",
            output_pattern_str,
        ])
        .output()
        .await;

    match result {
        Ok(output) if output.status.success() => {},
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return Err(format!("ffmpeg keyframe extraction failed: {}", stderr));
        }
        Err(e) => {
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return Err(format!("Failed to run ffmpeg: {}", e));
        }
    }

    let mut keyframes = Vec::new();
    let mut entries = tokio::fs::read_dir(&temp_dir)
        .await
        .map_err(|e| format!("Failed to read temp keyframes dir: {}", e))?;

    let mut frame_files = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("jpg") {
            frame_files.push(path);
        }
    }
    frame_files.sort();

    for (idx, path) in frame_files.into_iter().enumerate() {
        if idx >= max_keyframes {
            break;
        }
        let timestamp = (idx as f32) * interval_secs;
        if let Ok(bytes) = tokio::fs::read(&path).await {
            keyframes.push((timestamp, bytes));
        }
    }

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    info!("Extracted {} keyframes from video {:?}", keyframes.len(), video_path);
    Ok(keyframes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_validation() {
        let ok = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(is_valid_content_hash(ok));
        assert!(!is_valid_content_hash(""));
        assert!(!is_valid_content_hash(&ok[..63]), "too short rejected");
        assert!(!is_valid_content_hash(&ok.to_uppercase()), "uppercase rejected");
        assert!(
            !is_valid_content_hash("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"),
            "non-hex chars rejected"
        );
    }

    #[test]
    fn media_ext_validation() {
        assert!(is_valid_media_ext("jpg"));
        assert!(is_valid_media_ext("MP4"));
        assert!(!is_valid_media_ext(""));
        assert!(!is_valid_media_ext("a/b"), "path separator rejected");
        assert!(!is_valid_media_ext(".."), "dotdot rejected");
        assert!(!is_valid_media_ext(".hidden"), "leading dot rejected");
        assert!(!is_valid_media_ext("superlongext"), "too long rejected");
        assert!(!is_valid_media_ext("a b"), "whitespace rejected");
    }

    #[test]
    fn safe_resolve_rejects_traversal_and_missing_files() {
        assert!(safe_resolve_content_path("/tmp", "..", "jpg").is_err(), "traversal hash rejected");

        let valid_hash = "a".repeat(64);
        assert!(
            safe_resolve_content_path("/tmp", &valid_hash, "../x").is_err(),
            "traversal ext rejected"
        );
        assert!(
            safe_resolve_content_path("/tmp", &valid_hash, "jpg").is_err(),
            "nonexistent file must not silently succeed"
        );
    }

    #[test]
    fn subdirectory_path_uses_first_two_chars() {
        assert!(get_subdirectory_path("/base", "ab1234").ends_with("ab"));
        assert_eq!(get_subdirectory_path("/base", "a"), std::path::PathBuf::from("/base"));
    }

    #[test]
    fn parse_image_name_dates() {
        let dt = parse_date_from_image_name("IMG_20231222_191241.jpg").expect("camera-style name");
        assert_eq!(
            dt.to_rfc3339(),
            "2023-12-22T19:12:41+00:00",
            "IMG_YYYYMMDD_HHMMSS parses to a UTC datetime"
        );
        assert!(parse_date_from_image_name("IMG-20240115-WA0000.jpg").is_some(), "WhatsApp format");
        assert!(parse_date_from_image_name("IMG-20240115.jpg").is_some(), "date-only format");
        assert!(parse_date_from_image_name("photo.png").is_none());
        assert!(parse_date_from_image_name("VID_20240101_000000.mp4").is_none(), "video names excluded");
    }

    #[test]
    fn determines_media_type_from_name() {
        assert_eq!(determine_image_type("/storage/emulated/0/DCIM/Camera/IMG_1.jpg"), media::TYPE_CAMERA);
        assert_eq!(determine_image_type("WhatsApp Image x.jpg"), media::TYPE_WHATSAPP);
        assert_eq!(determine_image_type("Screenshot_1.png"), media::TYPE_SCREENSHOT);
        assert_eq!(determine_image_type("photo.png"), media::TYPE_OTHER);

        assert_eq!(determine_video_type("/DCIM/Camera/VID_1.mp4"), media::TYPE_CAMERA);
        assert_eq!(determine_video_type("DJI_0001.mp4"), media::TYPE_CAMERA);
        assert_eq!(determine_video_type("WhatsApp Video.mp4"), media::TYPE_WHATSAPP);
        assert_eq!(determine_video_type("screen recording.mp4"), media::TYPE_SCREEN_RECORDING);
        assert_eq!(determine_video_type("clip.mp4"), media::TYPE_OTHER);
    }

    #[test]
    fn parses_video_name_dates() {
        let dt = parse_date_from_video_name("VID_20240220_101530.mp4").expect("camera video");
        assert_eq!(dt.to_rfc3339(), "2024-02-20T10:15:30+00:00");
        assert!(parse_date_from_video_name("IMG_20240101_000000.jpg").is_none(), "image names excluded");
        assert!(parse_date_from_video_name("clip.mp4").is_none());
    }

    #[test]
    fn orient_image_to_jpeg_rejects_garbage() {
        assert!(orient_image_to_jpeg(b"not an image", 6).is_err());
    }

    #[test]
    fn rotate_png_rejects_garbage() {
        assert!(rotate_png_bytes(b"not a png", 6).is_none());
    }

    #[test]
    fn exif_orientation_round_trip() {
        // Not a JPEG -> no orientation, inject returns input unchanged.
        assert_eq!(read_exif_orientation_from_bytes(b"garbage"), None);
        let unchanged = inject_exif_orientation(b"garbage", 6);
        assert_eq!(unchanged, b"garbage".to_vec());

        // Minimal JPEG: SOI + a JFIF APP0 segment + EOI.
        let jpeg: Vec<u8> = vec![
            0xFF, 0xD8,
            0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x01,
            0xFF, 0xD9,
        ];

        // Inject orientation 6, then read it back.
        let out = inject_exif_orientation(&jpeg, 6);
        assert!(out.len() > jpeg.len(), "APP1 block appended");
        assert_eq!(&out[6..12], b"Exif\0\0");
        assert_eq!(read_exif_orientation_from_bytes(&out), Some(6), "round-trip orientation");

        // Already contains EXIF APP1 -> returned byte-for-byte unchanged.
        let again = inject_exif_orientation(&out, 8);
        assert_eq!(again, out);
    }

    #[actix_web::test]
    async fn hash_file_is_deterministic_64_hex() {
        let dir = std::env::temp_dir().join(format!("reminisce_hash_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f.bin");
        std::fs::write(&p, b"hello coverage bytes").unwrap();

        let h1 = hash_file_blake3(&p).await.expect("hash ok");
        let h2 = hash_file_blake3(&p).await.expect("hash ok");
        assert_eq!(h1.len(), 64);
        assert!(h1.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(h1, h2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn extract_video_keyframes_reports_missing_input() {
        let missing = std::env::temp_dir().join(format!("no_such_video_{}.mp4", std::process::id()));
        let res = extract_video_keyframes(&missing, 1.0, 3).await;
        assert!(res.is_err(), "missing input must be an error, got {:?}", res);
    }

}

