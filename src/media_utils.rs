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
    tokio::fs::create_dir_all(temp_dir).await
        .map_err(|_| actix_web::error::ErrorInternalServerError("Failed to create temp dir"))?;
    let temp_path = temp_dir.join(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut f = tokio::fs::File::create(&temp_path).await
        .map_err(|_| actix_web::error::ErrorInternalServerError("Failed to create temp file"))?;
    let mut hasher = blake3::Hasher::new();
    while let Ok(Some(chunk)) = field.try_next().await {
        hasher.update(&chunk);
        f.write_all(&chunk).await
            .map_err(|_| actix_web::error::ErrorInternalServerError("Failed to write temp file"))?;
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
        if media_type != media::TYPE_ALL {
            builder.with_media_type();
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
    if media_type != media::TYPE_ALL {
        params.push(&media_type as &(dyn tokio_postgres::types::ToSql + Sync));
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
        lat_value = location_lat.unwrap();
        lon_value = location_lon.unwrap();
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
        if media_type != media::TYPE_ALL {
            builder.with_media_type();
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
    if media_type != media::TYPE_ALL {
        params.push(&media_type as &(dyn tokio_postgres::types::ToSql + Sync));
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
        lat_value = location_lat.unwrap();
        lon_value = location_lon.unwrap();
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
