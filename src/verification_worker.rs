use super::utils::{ get_load_average, get_gpu_load, get_cpu_count, calculate_worker_concurrency };
use actix_web::web;
use log::{ error, info, warn };
use blake3::Hasher;
use std::time::Instant;
use tokio::io::AsyncReadExt;
use tokio::time::Duration;
use chrono::Utc;
use crate::config::Config;
use crate::db::MainDbPool;
use crate::metrics::{VERIFICATION_DURATION, VERIFICATION_SUCCESS_TOTAL, VERIFICATION_FAILURES_TOTAL, THUMBNAIL_PROCESSING_DELAY};
use crate::services::thumbnail::{generate_thumbnail_for_image, generate_thumbnail_for_video};
use futures::stream::StreamExt;

pub async fn start_verification_worker(pool: web::Data<MainDbPool>, config: web::Data<Config>) {
    info!("Verification worker started.");

    // Adaptive strategy:
    // - Active: config-based min interval
    // - Idle: config-based max interval
    let pool = pool.clone();
    let config_clone = config.clone();
    let min_dur = Duration::from_secs(config.workers.verification_min_secs);
    let max_dur = Duration::from_secs(config.workers.verification_max_secs);

    super::utils::run_worker_loop(
        "Verification Worker",
        min_dur,
        max_dur,
        move || {
            let pool = pool.clone();
            let config = config_clone.clone();
            async move { verify_files(pool, config).await }
        }
    ).await;
}

async fn verify_files(pool: web::Data<MainDbPool>, config: web::Data<Config>) -> Result<bool, String> {
    let client = pool.0.get().await.map_err(|e| format!("Failed to get database client: {}", e))?;

    let load_average = get_load_average().await;
    let gpu_load = get_gpu_load().await;
    let cpu_count = get_cpu_count();
    let limits = calculate_worker_concurrency(load_average, gpu_load, cpu_count);

    if limits.is_overloaded() {
        let normalized = load_average / (cpu_count as f64).max(1.0);
        info!("System load too high ({:.2} raw, {:.0}% normalized), skipping verification this cycle",
              load_average, normalized * 100.0);
        return Ok(false);
    }

    // Use verification concurrency for batch sizing (I/O bound, higher throughput)
    let batch_size: i64 = super::utils::calculate_parallel_batch_size(limits.verification, load_average, cpu_count);

    // Get distinct user IDs that have files needing verification or missing thumbnails
    let user_id_rows = client
        .query(
            "SELECT DISTINCT user_id FROM (\
                 SELECT user_id FROM images \
                 WHERE deleted_at IS NULL AND (verification_status = 0 OR verification_status = -1 \
                 OR (verification_status = 1 AND (last_verified_at IS NULL OR last_verified_at < NOW() - INTERVAL '1 month'))\
                 OR (verification_status = 1 AND has_thumbnail = false))\
                 UNION ALL \
                 SELECT user_id FROM videos \
                 WHERE deleted_at IS NULL AND (verification_status = 0 OR verification_status = -1 \
                 OR (verification_status = 1 AND (last_verified_at IS NULL OR last_verified_at < NOW() - INTERVAL '1 month'))\
                 OR (verification_status = 1 AND has_thumbnail = false))\
             ) AS users_to_verify",
            &[]
        ).await
        .map_err(|e| format!("Failed to query distinct user IDs for verification: {}", e))?;

    if user_id_rows.is_empty() {
        return Ok(false);
    }

    info!("Found {} distinct users with files to verify/process", user_id_rows.len());

    for user_id_row in user_id_rows {
        let current_user_id: uuid::Uuid = user_id_row.get(0);

        // Query for files (both images and videos) for the current user
        let file_rows = client
            .query(
                "(SELECT hash, ext, name, deviceid, 'image' as file_type, last_verified_at, has_thumbnail, created_at, orientation FROM images \
                 WHERE user_id = $1 AND deleted_at IS NULL AND (verification_status = 0 OR verification_status = -1 \
                 OR (verification_status = 1 AND (last_verified_at IS NULL OR last_verified_at < NOW() - INTERVAL '1 month'))\
                 OR (verification_status = 1 AND has_thumbnail = false))) \
                 UNION ALL \
                 (SELECT hash, ext, name, deviceid, 'video' as file_type, last_verified_at, has_thumbnail, created_at, NULL::SMALLINT as orientation FROM videos \
                 WHERE user_id = $1 AND deleted_at IS NULL AND (verification_status = 0 OR verification_status = -1 \
                 OR (verification_status = 1 AND (last_verified_at IS NULL OR last_verified_at < NOW() - INTERVAL '1 month'))\
                 OR (verification_status = 1 AND has_thumbnail = false))) \
                 ORDER BY last_verified_at ASC NULLS FIRST LIMIT $2",
                &[&current_user_id, &batch_size]
            ).await
            .map_err(|e| format!("Failed to query files for verification for user {}: {}", current_user_id, e))?;

        let total_files = file_rows.len();
        if total_files == 0 {
            continue;
        }

        info!("Found {} files to verify for user {}", total_files, current_user_id);

        let pool_clone = pool.clone();
        let config_clone = config.clone();

        let verification_stream = futures::stream::iter(file_rows.into_iter().enumerate()).map(|(index, row)| {
            let hash: String = row.get(0);
            let ext: String = row.get(1);
            let deviceid: String = row.get(3);
            let file_type: String = row.get(4);
            let mut has_thumbnail: bool = row.get(6);
            let created_at: chrono::DateTime<Utc> = row.get(7);
            let orientation: Option<i16> = row.get(8);
            let user_id = current_user_id;
            let total_files = total_files;

            let file_dir = if file_type == "image" { config_clone.get_images_dir().to_string() } else { config_clone.get_videos_dir().to_string() };
            let sub_dir_path = super::utils::get_subdirectory_path(&file_dir, &hash);
            let file_path = sub_dir_path.join(format!("{}.{}", hash, ext));

            let pool_inner = pool_clone.clone();

            async move {
                info!(
                    "Verifying {} {}/{}: {} (device: {}, thumbnail: {})",
                    file_type,
                    index + 1,
                    total_files,
                    hash,
                    deviceid,
                    has_thumbnail
                );

                let start_time = Instant::now();

                let client = match pool_inner.0.get().await {
                    Ok(c) => c,
                    Err(e) => {
                        error!("Failed to get database client for {}: {}", hash, e);
                        VERIFICATION_FAILURES_TOTAL.inc();
                        return;
                    }
                };

                match tokio::fs::File::open(&file_path).await {
                    Ok(mut file) => {
                        info!(
                            "Successfully opened {} file for verification: {}",
                            file_type,
                            file_path.display()
                        );
                        let mut hasher = Hasher::new();
                        let mut buffer = [0; 8192];
                        let mut read_failed = false;
                        loop {
                            match file.read(&mut buffer).await {
                                Ok(0) => break,
                                Ok(n) => {
                                    hasher.update(&buffer[..n]);
                                }
                                Err(e) => {
                                    error!("Failed to read {} file chunk for verification {}: {}", file_type, hash, e);
                                    let table_name = if file_type == "image" { "images" } else { "videos" };
                                    if let Ok(_) = crate::utils::validate_table_name(table_name) {
                                        let query = format!("UPDATE {} SET last_verified_at = NOW(), verification_status = -1 WHERE hash = $1 AND user_id = $2", table_name);
                                        let _ = client.execute(&query, &[&hash, &user_id]).await;
                                    }
                                    read_failed = true;
                                    break;
                                }
                            }
                        }
                        if read_failed {
                            VERIFICATION_FAILURES_TOTAL.inc();
                            return;
                        }

                        let calculated_hash = hasher.finalize().to_hex().to_string();
                        if calculated_hash == hash {
                            let duration = start_time.elapsed();
                            VERIFICATION_DURATION.observe(duration.as_secs_f64());
                            VERIFICATION_SUCCESS_TOTAL.inc();
                            info!("{} verification successful for hash: {} (took {:.2}s)", file_type, hash, duration.as_secs_f64());
                            
                            if !has_thumbnail {
                                let thumb_filename = format!("{}.thumb.jpg", hash);
                                let thumb_path = sub_dir_path.join(&thumb_filename);
                                
                                info!("Generating missing thumbnail for {} {}", file_type, hash);
                                let generation_result = if file_type == "image" {
                                    generate_thumbnail_for_image(&file_path, &thumb_path, 500, orientation).await
                                } else {
                                    generate_thumbnail_for_video(&file_path, &thumb_path).await
                                };
                                
                                match generation_result {
                                    Ok(_) => {
                                        info!("Successfully generated missing thumbnail for {} {}", file_type, hash);
                                        has_thumbnail = true;
                                        let delay = Utc::now().signed_duration_since(created_at);
                                        let delay_secs = delay.num_seconds().max(0) as f64;
                                        THUMBNAIL_PROCESSING_DELAY.observe(delay_secs);
                                    }
                                    Err(e) => {
                                        error!("Failed to generate missing thumbnail for {} {}: {}", file_type, hash, e);
                                    }
                                }
                            }

                            let table_name = if file_type == "image" { "images" } else { "videos" };
                            if let Ok(_) = crate::utils::validate_table_name(table_name) {
                                let query = format!("UPDATE {} SET last_verified_at = NOW(), verification_status = 1, has_thumbnail = $3 WHERE hash = $1 AND user_id = $2", table_name);
                                let _ = client.execute(&query, &[&hash, &user_id, &has_thumbnail]).await;
                            }
                        } else {
                            let duration = start_time.elapsed();
                            VERIFICATION_DURATION.observe(duration.as_secs_f64());
                            VERIFICATION_FAILURES_TOTAL.inc();
                            warn!("{} verification failed for hash: {}. Expected {}, got {} (took {:.2}s)", file_type, hash, hash, calculated_hash, duration.as_secs_f64());
                            let table_name = if file_type == "image" { "images" } else { "videos" };
                            if let Ok(_) = crate::utils::validate_table_name(table_name) {
                                let query = format!("UPDATE {} SET last_verified_at = NOW(), verification_status = -1 WHERE hash = $1 AND user_id = $2", table_name);
                                let _ = client.execute(&query, &[&hash, &user_id]).await;
                            }
                        }
                    }
                    Err(e) => {
                        VERIFICATION_FAILURES_TOTAL.inc();
                        error!("Failed to open {} file for verification {}: {}", file_type, hash, e);
                        let table_name = if file_type == "image" { "images" } else { "videos" };
                        if let Ok(_) = crate::utils::validate_table_name(table_name) {
                            let query = format!("UPDATE {} SET last_verified_at = NOW(), verification_status = -1 WHERE hash = $1 AND user_id = $2", table_name);
                            let _ = client.execute(&query, &[&hash, &user_id]).await;
                        }
                    }
                }
            }
        });

        let mut buffered = verification_stream.buffer_unordered(limits.verification);
        while let Some(_) = buffered.next().await {}
        info!("All verification tasks completed for user {}", current_user_id);
    }

    Ok(true)
}