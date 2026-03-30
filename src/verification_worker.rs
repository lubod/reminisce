use super::utils::{ get_load_average, get_gpu_load, get_cpu_count, calculate_worker_concurrency };
use actix_web::web;
use log::{ error, info, warn };
use blake3::Hasher;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::AsyncReadExt;
use tokio::sync::Semaphore;
use tokio::time::Duration;
use chrono::Utc;
use crate::config::Config;
use crate::db::MainDbPool;
use crate::metrics::{VERIFICATION_DURATION, VERIFICATION_SUCCESS_TOTAL, VERIFICATION_FAILURES_TOTAL, THUMBNAIL_PROCESSING_DELAY};
use crate::services::thumbnail::{generate_thumbnail_for_image, generate_thumbnail_for_video};

pub async fn start_verification_worker(pool: web::Data<MainDbPool>, config: web::Data<Config>) {
    info!("Verification worker started.");

    // Adaptive strategy:
    // - Active: 1s (Process queue quickly)
    // - Idle: Backoff up to 10s
    super::utils::run_worker_loop(
        "Verification Worker",
        Duration::from_secs(1),
        Duration::from_secs(10),
        || verify_files(pool.clone(), config.clone())
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

    // Only log if we are actually going to check DB (reduce log noise)
    // info!("System load: {:.2}...", ...); 

    // First, get distinct device IDs that have files needing verification or missing thumbnails
    let device_id_rows = client
        .query(
            "SELECT DISTINCT deviceid FROM (\n                 SELECT deviceid FROM images \n                 WHERE deleted_at IS NULL AND (verification_status = 0 OR verification_status = -1 \n                 OR (verification_status = 1 AND (last_verified_at IS NULL OR last_verified_at < NOW() - INTERVAL '1 month'))\n                 OR (verification_status = 1 AND has_thumbnail = false))\n                 UNION ALL \n                 SELECT deviceid FROM videos \n                 WHERE deleted_at IS NULL AND (verification_status = 0 OR verification_status = -1 \n                 OR (verification_status = 1 AND (last_verified_at IS NULL OR last_verified_at < NOW() - INTERVAL '1 month'))\n                 OR (verification_status = 1 AND has_thumbnail = false))\n             ) AS devices_to_verify;",
            &[]
        ).await
        .map_err(|e| format!("Failed to query distinct device IDs for verification: {}", e))?;

    if device_id_rows.is_empty() {
        return Ok(false);
    }

    info!("Found {} distinct device IDs with files to verify/process", device_id_rows.len());

    for device_id_row in device_id_rows {
        let current_device_id: String = device_id_row.get(0);
        // info!("Processing files for device ID: {}", current_device_id);

        // Query for files (both images and videos) for the current device ID
        let file_rows = client
            .query(
                "(SELECT hash, ext, name, deviceid, 'image' as file_type, last_verified_at, has_thumbnail, created_at, orientation FROM images \n                 WHERE deviceid = $1 AND deleted_at IS NULL AND (verification_status = 0 OR verification_status = -1 \n                 OR (verification_status = 1 AND (last_verified_at IS NULL OR last_verified_at < NOW() - INTERVAL '1 month'))\n                 OR (verification_status = 1 AND has_thumbnail = false))) \n                 UNION ALL \n                 (SELECT hash, ext, name, deviceid, 'video' as file_type, last_verified_at, has_thumbnail, created_at, NULL::SMALLINT as orientation FROM videos \n                 WHERE deviceid = $1 AND deleted_at IS NULL AND (verification_status = 0 OR verification_status = -1 \n                 OR (verification_status = 1 AND (last_verified_at IS NULL OR last_verified_at < NOW() - INTERVAL '1 month'))\n                 OR (verification_status = 1 AND has_thumbnail = false))) \n                 ORDER BY last_verified_at ASC NULLS FIRST LIMIT $2;",
                &[&current_device_id, &batch_size]
            ).await
            .map_err(|e| format!("Failed to query files for verification for device {}: {}", current_device_id, e))?;

        let total_files = file_rows.len();
        if total_files == 0 {
            continue;
        }
        
        info!("Found {} files to verify for device {}", total_files, current_device_id);

        // Verification is I/O-bound (BLAKE3 hashing), use calculated concurrency
        let hash_verification_semaphore = Arc::new(Semaphore::new(limits.verification));
        
        // Spawn concurrent verification tasks
        let mut tasks = Vec::new();

        for (index, row) in file_rows.into_iter().enumerate() {
            let hash: String = row.get(0);
            let ext: String = row.get(1);
            let _name: String = row.get(2);
            let deviceid: String = row.get(3);
            let file_type: String = row.get(4);
            let mut has_thumbnail: bool = row.get(6);
            let created_at: chrono::DateTime<Utc> = row.get(7);
            let orientation: Option<i16> = row.get(8); // NULL for videos

            let file_dir = if file_type == "image" { config.get_images_dir().to_string() } else { config.get_videos_dir().to_string() };
            let sub_dir_path = super::utils::get_subdirectory_path(&file_dir, &hash);
            let file_path = sub_dir_path.join(format!("{}.{}", hash, ext));

            // Clone resources for the task
            let pool_clone = pool.clone();
            let hash_sem_clone = hash_verification_semaphore.clone();

            // Spawn a task for each file
            let task = tokio::spawn(async move {
                // Acquire semaphore permit for hash verification (limits concurrency)
                let _permit = hash_sem_clone.acquire().await
                    .expect("hash_verification_semaphore closed unexpectedly");

                info!(
                    "Verifying {} {}/{}: {} (device: {}, thumbnail: {})",
                    file_type,
                    index + 1,
                    total_files,
                    hash,
                    deviceid,
                    has_thumbnail
                );

                // Start timing for verification
                let start_time = Instant::now();

                // Get a database client for this task
                let client = match pool_clone.0.get().await {
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
                    let mut buffer = [0; 8192]; // 8KB buffer for better I/O performance
                    loop {
                        match file.read(&mut buffer).await {
                            Ok(0) => {
                                break;
                            } // End of file
                            Ok(n) => {
                                hasher.update(&buffer[..n]);
                            }
                            Err(e) => {
                                error!(
                                    "Failed to read {} file chunk for verification {}: {}",
                                    file_type,
                                    hash,
                                    e
                                );
                                // Update verification status to -1 (failed)
                                let table_name = if file_type == "image" { "images" } else { "videos" };
                                let query =
                                    format!("UPDATE {} SET last_verified_at = NOW(), verification_status = -1 WHERE hash = $1 AND deviceid = $2", table_name);
                                if let Err(db_err) = client.execute(&query, &[&hash, &deviceid]).await {
                                    error!(
                                        "Failed to update verification_status for {} {}: {}",
                                        file_type,
                                        hash,
                                        db_err
                                    );
                                }
                                break;
                            }
                        }
                    }
                    let calculated_hash = hasher.finalize().to_hex().to_string();
                    if calculated_hash == hash {
                        let duration = start_time.elapsed();
                        VERIFICATION_DURATION.observe(duration.as_secs_f64());
                        VERIFICATION_SUCCESS_TOTAL.inc();
                        info!("{} verification successful for hash: {} (took {:.2}s)", file_type, hash, duration.as_secs_f64());
                        
                        // Check if we need to generate thumbnail
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
                                    
                                    // Record processing delay
                                    let delay = Utc::now().signed_duration_since(created_at);
                                    let delay_secs = delay.num_seconds().max(0) as f64;
                                    THUMBNAIL_PROCESSING_DELAY.observe(delay_secs);
                                }
                                Err(e) => {
                                    error!("Failed to generate missing thumbnail for {} {}: {}", file_type, hash, e);
                                    // Don't fail the verification just because thumbnail failed, 
                                    // but we won't set has_thumbnail=true
                                }
                            }
                        }

                        // Mark as verified immediately and update thumbnail status
                        let table_name = if file_type == "image" { "images" } else { "videos" };
                        let query = format!("UPDATE {} SET last_verified_at = NOW(), verification_status = 1, has_thumbnail = $3 WHERE hash = $1 AND deviceid = $2", table_name);
                        if let Err(e) = client.execute(&query, &[&hash, &deviceid, &has_thumbnail]).await {
                            error!("Failed to update verification_status for {} {}: {}", file_type, hash, e);
                        }

                        // NOTE: Replication/Backup functionality has been removed.
                        // Previously, we would upload the verified file to another server here.

                    } else {
                        let duration = start_time.elapsed();
                        VERIFICATION_DURATION.observe(duration.as_secs_f64());
                        VERIFICATION_FAILURES_TOTAL.inc();
                        warn!(
                            "{} verification failed for hash: {}. Expected {}, got {} (took {:.2}s)",
                            file_type,
                            hash,
                            hash,
                            calculated_hash,
                            duration.as_secs_f64()
                        );
                        info!("Skipping AI description and embedding generation for unverified file: {}", hash);
                        // Update verification status to -1 (failed)
                        let table_name = if file_type == "image" { "images" } else { "videos" };
                        let query =
                            format!("UPDATE {} SET last_verified_at = NOW(), verification_status = -1 WHERE hash = $1 AND deviceid = $2", table_name);
                        if let Err(e) = client.execute(&query, &[&hash, &deviceid]).await {
                            error!(
                                "Failed to update verification_status for failed {} {}: {}",
                                file_type,
                                hash,
                                e
                            );
                        }
                    }
                }
                Err(e) => {
                    VERIFICATION_FAILURES_TOTAL.inc();
                    error!("Failed to open {} file for verification {}: {}", file_type, hash, e);
                    // Update verification status to -1 (failed)
                    let table_name = if file_type == "image" { "images" } else { "videos" };
                    let query =
                        format!("UPDATE {} SET last_verified_at = NOW(), verification_status = -1 WHERE hash = $1 AND deviceid = $2", table_name);
                    if let Err(db_err) = client.execute(&query, &[&hash, &deviceid]).await {
                        error!(
                            "Failed to update verification_status for failed-to-open {} {}: {}",
                            file_type,
                            hash,
                            db_err
                        );
                    }
                }
            }
            });

            tasks.push(task);
        }

        // Wait for all tasks to complete
        info!("Waiting for {} verification tasks to complete for device {}...", tasks.len(), current_device_id);
        for task in tasks {
            if let Err(e) = task.await {
                error!("Verification task failed: {}", e);
            }
        }
        info!("All verification tasks completed for device {}", current_device_id);
    }

    Ok(true)
}