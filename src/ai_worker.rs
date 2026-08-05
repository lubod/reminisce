use super::utils::{get_load_average, get_cpu_count, calculate_worker_concurrency};
use crate::config::Config;
use crate::db::MainDbPool;
use crate::metrics::{
    AI_DESCRIPTION_DURATION, AI_DESCRIPTION_SUCCESS_TOTAL, AI_DESCRIPTION_FAILURES_TOTAL,
    EMBEDDING_DURATION, EMBEDDING_SUCCESS_TOTAL, EMBEDDING_FAILURES_TOTAL,
    FACE_DETECTION_DURATION, FACE_DETECTION_SUCCESS_TOTAL, FACE_DETECTION_FAILURES_TOTAL,
    FACES_DETECTED_TOTAL, FACE_CLUSTERING_DURATION,
    AI_DESCRIPTION_PROCESSING_DELAY, EMBEDDING_PROCESSING_DELAY, FACE_DETECTION_PROCESSING_DELAY,
    ORIENTATION_DURATION, ORIENTATION_SUCCESS_TOTAL, ORIENTATION_FAILURES_TOTAL,
    ORIENTATION_PROCESSING_DELAY,
    TOTAL_IMAGES, IMAGES_WITH_EMBEDDING, IMAGES_WITH_DESCRIPTION, IMAGES_FACE_PROCESSED,
    THUMBNAIL_COUNT, THUMBNAIL_COVERAGE,
};
use actix_web::web;
use log::{error, info, warn};
use std::sync::LazyLock as Lazy;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::sync::Semaphore;
use tokio::time::Duration;
use futures::stream::{self, StreamExt};
use chrono::{DateTime, Utc};

#[allow(dead_code)]
struct AiTask {
    hash: String,
    ext: String,
    name: String,
    user_id: uuid::Uuid,
    file_type: String,
    process_description: bool,
    process_embedding: bool,
    process_faces: bool,
    process_quality: bool,
    process_orientation: bool,
    created_at: DateTime<Utc>,
    orientation: Option<i16>,
}

/// Resize image to fit within max_dim on longest side, preserving aspect ratio.
/// Returns JPEG bytes. Runs in blocking thread since image decoding is CPU-bound.
async fn resize_image_for_ai(image_data: Vec<u8>, max_dim: u32) -> Result<Vec<u8>, String> {
    actix_web::web::block(move || {
        let img = image::load_from_memory(&image_data)
            .map_err(|e| format!("Failed to decode image: {}", e))?;
        let (w, h) = (img.width(), img.height());
        if w <= max_dim && h <= max_dim {
            return Ok(image_data); // Already small enough
        }
        let resized = img.resize(max_dim, max_dim, image::imageops::FilterType::Lanczos3);
        let mut buf = std::io::Cursor::new(Vec::new());
        resized.write_to(&mut buf, image::ImageOutputFormat::Jpeg(90))
            .map_err(|e| format!("Failed to encode resized image: {}", e))?;
        Ok(buf.into_inner())
    }).await
        .map_err(|e| format!("Blocking task failed: {}", e))?
}

pub async fn start_ai_worker(
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
    shutdown_token: tokio_util::sync::CancellationToken,
) {
    info!("AI worker started.");
    
    let pool = pool.clone();
    let config_clone = config.clone();
    let min_dur = Duration::from_secs(config.workers.ai_min_secs);
    let max_dur = Duration::from_secs(config.workers.ai_max_secs);
    super::utils::run_worker_loop(
        "AI Worker",
        min_dur,
        max_dur,
        shutdown_token,
        move || {
            let pool = pool.clone();
            let config = config_clone.clone();
            async move { process_files(pool, config).await }
        }
    ).await;
}

async fn process_files(pool: web::Data<MainDbPool>, config: web::Data<Config>) -> Result<bool, String> {
    // Periodically update overall library status metrics
    static LAST_STATUS_UPDATE: Lazy<std::sync::Mutex<Option<Instant>>> = Lazy::new(|| std::sync::Mutex::new(None));
    
    let should_update = {
        // Recover from a poisoned mutex rather than panicking — this is only a
        // 30s metrics-throttle timestamp, so a prior panic shouldn't kill the worker.
        let mut last_update = LAST_STATUS_UPDATE.lock().unwrap_or_else(|e| e.into_inner());
        match *last_update {
            Some(last) if last.elapsed() < Duration::from_secs(30) => false,
            _ => {
                *last_update = Some(Instant::now());
                true
            }
        }
    };

    if should_update {
        if let Ok(client) = pool.0.get().await {
            let _ = update_status_metrics(&client).await;
        }
    }

    // Check if AI processing is enabled
    let enable_ai_descriptions = config.enable_ai_descriptions.load(std::sync::atomic::Ordering::Relaxed);
    let enable_embeddings = config.enable_embeddings.load(std::sync::atomic::Ordering::Relaxed);
    let config_embedding_limit = config.embedding_parallel_count.load(std::sync::atomic::Ordering::Relaxed);
    let enable_face_detection = config.enable_face_detection.load(std::sync::atomic::Ordering::Relaxed);
    let config_face_limit = config.face_detection_parallel_count.load(std::sync::atomic::Ordering::Relaxed);
    let enable_orientation_detection = config.enable_orientation_detection.load(std::sync::atomic::Ordering::Relaxed);
    let config_orientation_limit = config.orientation_detection_parallel_count.load(std::sync::atomic::Ordering::Relaxed);

    if !enable_ai_descriptions && !enable_embeddings && !enable_face_detection && !enable_orientation_detection {
        info!("AI descriptions, embeddings, face detection, and orientation detection are all disabled, skipping this cycle.");
        return Ok(false);
    }

    let client = pool.0.get().await.map_err(|e| format!("Failed to get database client: {}", e))?;

    let load_average = get_load_average().await;
    let gpu_load = super::utils::get_gpu_load().await;
    let cpu_count = get_cpu_count();
    let limits = calculate_worker_concurrency(load_average, gpu_load, cpu_count);

    if limits.is_overloaded() {
        let normalized = load_average / (cpu_count as f64).max(1.0);
        info!("System load too high ({:.2} raw, {:.0}% normalized), skipping AI processing this cycle",
              load_average, normalized * 100.0);
        return Ok(false);
    }

    if limits.gpu_overloaded {
        info!("GPU load too high ({}%), skipping AI processing this cycle", gpu_load);
        return Ok(false);
    }

    // Use weighted concurrency limits based on priority: embedding > face > description
    let embedding_concurrency = limits.embedding.min(config_embedding_limit);
    let face_concurrency = limits.face_detection.min(config_face_limit);
    let orientation_concurrency = limits.embedding.min(config_orientation_limit);
    let description_concurrency = limits.description;

    // We use a total task limit to prevent fetching too many tasks at once
    let total_batch_limit = (embedding_concurrency + orientation_concurrency + face_concurrency + description_concurrency) * 2;
    let mut all_tasks_to_process = std::collections::HashMap::new();

    // --- STRICT PRIORITY FETCHING ---
    
    // 1. HIGH PRIORITY: Embeddings
    if enable_embeddings {
        let embedding_rows = client
            .query(
                "SELECT hash, ext, name, user_id, 'image' as file_type, created_at FROM images
                 WHERE verification_status = 1 AND deleted_at IS NULL AND embedding IS NULL AND embedding_generated_at IS NULL
                   AND lower(ext) != 'svg'
                 LIMIT $1",
                &[&(total_batch_limit as i64)],
            )
            .await
            .map_err(|e| format!("Failed to query for AI embedding tasks: {}", e))?;

        for row in embedding_rows {
            let hash: String = row.get(0);
            let ext: String = row.get(1);
            let name: String = row.get(2);
            let user_id: uuid::Uuid = row.get(3);
            let file_type: String = row.get(4);
            let created_at: DateTime<Utc> = row.get(5);
            all_tasks_to_process.insert(hash.clone(), AiTask {
                hash: hash.clone(),
                ext,
                name,
                user_id,
                file_type,
                process_description: false,
                process_embedding: true,
                process_faces: false,
                process_quality: false,
                process_orientation: false,
                created_at,
                orientation: None,
            });
        }

        if all_tasks_to_process.len() < total_batch_limit {
            let room_left = total_batch_limit - all_tasks_to_process.len();
            let video_embedding_rows = client
                .query(
                    "SELECT hash, ext, name, user_id, 'video' as file_type, created_at FROM videos
                     WHERE verification_status = 1 AND deleted_at IS NULL AND embedding IS NULL AND embedding_generated_at IS NULL
                     LIMIT $1",
                    &[&(room_left as i64)],
                )
                .await
                .map_err(|e| format!("Failed to query for video AI embedding tasks: {}", e))?;

            for row in video_embedding_rows {
                let hash: String = row.get(0);
                let ext: String = row.get(1);
                let name: String = row.get(2);
                let user_id: uuid::Uuid = row.get(3);
                let file_type: String = row.get(4);
                let created_at: DateTime<Utc> = row.get(5);
                all_tasks_to_process.entry(hash.clone()).or_insert(AiTask {
                    hash: hash.clone(),
                    ext,
                    name,
                    user_id,
                    file_type,
                    process_description: false,
                    process_embedding: true,
                    process_faces: false,
                    process_quality: false,
                    process_orientation: false,
                    created_at,
                    orientation: None,
                });
            }
        }
    }

    // 2. MEDIUM PRIORITY: Face Detection
    // Only fetch if we have room in our batch limit
    if enable_face_detection && all_tasks_to_process.len() < total_batch_limit {
        let room_left = total_batch_limit - all_tasks_to_process.len();
        let face_detection_rows = client
            .query(
                "SELECT i.hash, i.ext, i.name, i.user_id, 'image' as file_type, i.created_at, i.orientation
                 FROM images i
                 WHERE i.verification_status = 1
                   AND i.deleted_at IS NULL
                   AND i.face_detection_completed_at IS NULL
                   AND lower(i.ext) != 'svg'
                 LIMIT $1",
                &[&(room_left as i64)],
            )
            .await
            .map_err(|e| format!("Failed to query for face detection tasks: {}", e))?;

        for row in face_detection_rows {
            let hash: String = row.get(0);
            let ext: String = row.get(1);
            let name: String = row.get(2);
            let user_id: uuid::Uuid = row.get(3);
            let file_type: String = row.get(4);
            let created_at: DateTime<Utc> = row.get(5);
            let orientation: Option<i16> = row.try_get(6).unwrap_or(None);

            all_tasks_to_process.entry(hash.clone())
                .and_modify(|e| {
                    e.process_faces = true;
                    e.orientation = orientation;
                })
                .or_insert(AiTask {
                    hash: hash.clone(),
                    ext,
                    name,
                    user_id,
                    file_type,
                    process_description: false,
                    process_embedding: false,
                    process_faces: true,
                    process_quality: false,
                    process_orientation: false,
                    created_at,
                    orientation,
                });
        }
    }

    // 2.5. HIGH-MEDIUM PRIORITY: AI-fallback orientation detection
    // Only for images with NO EXIF (exif column NULL means the file carries no EXIF to
    // read orientation from) that have not yet been detected. The classifier result is
    // stored as an EXIF orientation value so the serving/thumbnail pipeline right the
    // photo (see media.rs inject_exif_orientation).
    if enable_orientation_detection && all_tasks_to_process.len() < total_batch_limit {
        let room_left = total_batch_limit - all_tasks_to_process.len();
        let orientation_rows = client
            .query(
                "SELECT hash, ext, name, user_id, 'image' as file_type, created_at, orientation
                 FROM images
                 WHERE verification_status = 1
                   AND deleted_at IS NULL
                   AND exif IS NULL
                   AND orientation IS NULL
                   AND orientation_detected_at IS NULL
                   AND lower(ext) != 'svg'
                 LIMIT $1",
                &[&(room_left as i64)],
            )
            .await
            .map_err(|e| format!("Failed to query for orientation detection tasks: {}", e))?;

        for row in orientation_rows {
            let hash: String = row.get(0);
            let ext: String = row.get(1);
            let name: String = row.get(2);
            let user_id: uuid::Uuid = row.get(3);
            let file_type: String = row.get(4);
            let created_at: DateTime<Utc> = row.get(5);
            let orientation: Option<i16> = row.try_get(6).unwrap_or(None);

            all_tasks_to_process.entry(hash.clone())
                .and_modify(|e| e.process_orientation = orientation.is_none())
                .or_insert(AiTask {
                    hash: hash.clone(),
                    ext,
                    name,
                    user_id,
                    file_type,
                    process_description: false,
                    process_embedding: false,
                    process_faces: false,
                    process_quality: false,
                    process_orientation: true,
                    created_at,
                    orientation,
                });
        }
    }

    // 3. LOW PRIORITY: Descriptions
    // Only fetch if we still have room AND GPU is not at absolute capacity (> 95%)
    if enable_ai_descriptions && all_tasks_to_process.len() < total_batch_limit && gpu_load < 95 {
        let room_left = total_batch_limit - all_tasks_to_process.len();
        let description_rows = client
            .query(
                "SELECT hash, ext, name, user_id, 'image' as file_type, created_at FROM images
                 WHERE verification_status = 1 AND deleted_at IS NULL AND description IS NULL
                   AND lower(ext) != 'svg'
                 LIMIT $1",
                &[&(room_left as i64)],
            )
            .await
            .map_err(|e| format!("Failed to query for AI description tasks: {}", e))?;

        for row in description_rows {
            let hash: String = row.get(0);
            let ext: String = row.get(1);
            let name: String = row.get(2);
            let user_id: uuid::Uuid = row.get(3);
            let file_type: String = row.get(4);
            let created_at: DateTime<Utc> = row.get(5);

            all_tasks_to_process.entry(hash.clone())
                .and_modify(|e| e.process_description = true)
                .or_insert(AiTask {
                    hash: hash.clone(),
                    ext,
                    name,
                    user_id,
                    file_type,
                    process_description: true,
                    process_embedding: false,
                    process_faces: false,
                    process_quality: false,
                    process_orientation: false,
                    created_at,
                    orientation: None,
                });
        }
    }

    // 4. LOWEST PRIORITY: Quality scoring
    // Only for images that already have embeddings; skip if system is busy with higher-priority tasks
    if all_tasks_to_process.len() < total_batch_limit {
        let room_left = total_batch_limit - all_tasks_to_process.len();
        let quality_rows = client
            .query(
                "SELECT hash, ext, name, user_id, 'image' as file_type, created_at FROM images
                 WHERE verification_status = 1
                   AND deleted_at IS NULL
                   AND embedding IS NOT NULL
                   AND quality_score_generated_at IS NULL
                   AND lower(ext) != 'svg'
                 LIMIT $1",
                &[&(room_left as i64)],
            )
            .await
            .map_err(|e| format!("Failed to query for quality tasks: {}", e))?;

        for row in quality_rows {
            let hash: String = row.get(0);
            let ext: String = row.get(1);
            let name: String = row.get(2);
            let user_id: uuid::Uuid = row.get(3);
            let file_type: String = row.get(4);
            let created_at: DateTime<Utc> = row.get(5);

            all_tasks_to_process.entry(hash.clone())
                .and_modify(|e| e.process_quality = true)
                .or_insert(AiTask {
                    hash: hash.clone(),
                    ext,
                    name,
                    user_id,
                    file_type,
                    process_description: false,
                    process_embedding: false,
                    process_faces: false,
                    process_quality: true,
                    process_orientation: false,
                    created_at,
                    orientation: None,
                });
        }
    }

    if all_tasks_to_process.is_empty() {
        return Ok(false);
    }

    let tasks_to_process: Vec<_> = all_tasks_to_process.into_values().collect();
    let total_files = tasks_to_process.len();

    // Concurrency limit for the parallel stream
    let quality_concurrency = description_concurrency; // Same as description: lowest priority
    let concurrent_limit = (description_concurrency + embedding_concurrency + orientation_concurrency + face_concurrency + quality_concurrency) * 2;

    info!("AI cycle: Processing {} files [CPU Load: {:.1}, GPU Load: {}%]",
          total_files, load_average, gpu_load);

    // Semaphores enforce priority through weighted concurrency:
    let description_semaphore = Arc::new(Semaphore::new(description_concurrency));
    let embedding_semaphore = Arc::new(Semaphore::new(embedding_concurrency));
    let orientation_semaphore = Arc::new(Semaphore::new(orientation_concurrency));
    let face_semaphore = Arc::new(Semaphore::new(face_concurrency));
    let quality_semaphore = Arc::new(Semaphore::new(quality_concurrency));

    // Track users that had faces detected for batch clustering at the end
    let users_with_new_faces = Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new()));
    let users_with_new_faces_clone = users_with_new_faces.clone();

    stream::iter(tasks_to_process)
        .for_each_concurrent(concurrent_limit, move |task| {
            let hash = task.hash;
            let ext = task.ext;
            let user_id = task.user_id;
            let file_type = task.file_type;
            let process_description = task.process_description;
            let process_embedding = task.process_embedding;
            let process_faces = task.process_faces;
            let process_quality = task.process_quality;
            let process_orientation = task.process_orientation;
            let created_at = task.created_at;
            let db_orientation = task.orientation;

            let file_dir = if file_type == "image" { config.get_images_dir().to_string() } else { config.get_videos_dir().to_string() };
            let sub_dir_path = super::utils::get_subdirectory_path(&file_dir, &hash);
            let file_path = sub_dir_path.join(format!("{}.{}", hash, ext));

            let pool_clone = pool.clone();
            let config_clone = config.clone();
            let desc_sem_clone = description_semaphore.clone();
            let emb_sem_clone = embedding_semaphore.clone();
            let orient_sem_clone = orientation_semaphore.clone();
            let face_sem_clone = face_semaphore.clone();
            let quality_sem_clone = quality_semaphore.clone();
            let users_set_clone = users_with_new_faces_clone.clone();

            async move {
                let client = match pool_clone.0.get().await {
                    Ok(c) => c,
                    Err(e) => {
                        error!("Failed to get database client for {}: {}", hash, e);
                        return;
                    }
                };

                let delay_secs = Utc::now().signed_duration_since(created_at).num_seconds().max(0) as f64;

                if process_description {
                    // Description is lowest priority - acquire permit
                    let _desc_permit = match desc_sem_clone.acquire().await {
                        Ok(p) => p,
                        Err(e) => {
                            warn!("Description semaphore closed for {}: {}", hash, e);
                            return;
                        }
                    };
                    info!("Starting AI description for {} : {}", file_type, hash);
                    let start_time = Instant::now();
                    match get_image_description(&file_path, &hash, &file_type, &config_clone).await {
                        Ok(desc) if !desc.is_empty() => {
                            let duration = start_time.elapsed();
                            AI_DESCRIPTION_DURATION.observe(duration.as_secs_f64());
                            AI_DESCRIPTION_SUCCESS_TOTAL.inc();
                            AI_DESCRIPTION_PROCESSING_DELAY.observe(delay_secs);
                            info!("Got AI description for {} {} (took {:.2}s): {}", file_type, hash, duration.as_secs_f64(), desc);
                            let table_name = if file_type == "image" { "images" } else { "videos" };
                            if let Err(e) = crate::utils::validate_table_name(table_name) {
                                error!("Table name validation failed for {}: {}", table_name, e);
                                return;
                            }
                            let query = format!("UPDATE {} SET description = $1 WHERE hash = $2 AND user_id = $3", table_name);
                            if let Err(e) = client.execute(&query, &[&desc, &hash, &user_id]).await {
                                error!("Failed to update description for {} {}: {}", file_type, hash, e);
                            }
                        }
                        // Empty generation output is treated as a permanent skip so the
                        // row isn't re-fetched and re-sent to the AI service every cycle.
                        Ok(_) => {
                            let table_name = if file_type == "image" { "images" } else { "videos" };
                            if let Err(e) = crate::utils::validate_table_name(table_name) {
                                error!("Table name validation failed for {}: {}", table_name, e);
                                return;
                            }
                            warn!("AI returned empty description for {} {}, marking as skipped to avoid infinite retry", file_type, hash);
                            let query = format!("UPDATE {} SET description = $1 WHERE hash = $2 AND user_id = $3", table_name);
                            let _ = client.execute(&query, &[&"[skipped]", &hash, &user_id]).await;
                        }
                        Err(e) => {
                            let duration = start_time.elapsed();
                            AI_DESCRIPTION_DURATION.observe(duration.as_secs_f64());
                            AI_DESCRIPTION_FAILURES_TOTAL.inc();
                            error!("Failed to get AI description for {} {} (took {:.2}s): {}", file_type, hash, duration.as_secs_f64(), e);
                            // Mark permanent failures (invalid input / auth errors) so
                            // they aren't retried forever.
                            if crate::ai_client::is_permanent_failure(&e) {
                                let table_name = if file_type == "image" { "images" } else { "videos" };
                                if let Err(e) = crate::utils::validate_table_name(table_name) {
                                    error!("Table name validation failed for {}: {}", table_name, e);
                                    return;
                                }
                                let query = format!("UPDATE {} SET description = $1 WHERE hash = $2 AND user_id = $3", table_name);
                                let _ = client.execute(&query, &[&"[skipped]", &hash, &user_id]).await;
                            }
                        }
                    }
                    // _ai_permit automatically dropped here
                }

                if process_embedding {
                    // Acquire permit in scope to auto-release when done
                    let _emb_permit = match emb_sem_clone.acquire().await {
                        Ok(p) => p,
                        Err(e) => {
                            warn!("Embedding semaphore closed for {}: {}", hash, e);
                            return;
                        }
                    };
                    info!("Starting embedding generation for {}: {}", file_type, hash);
                    let start_time = Instant::now();
                    let res = if file_type == "video" {
                        generate_and_store_video_embedding(&hash, &file_path, &user_id, &config_clone, &client).await
                    } else {
                        generate_and_store_embedding(&hash, &file_path, &user_id, &config_clone, &client).await
                    };
                    match res {
                        Ok(_) => {
                            let duration = start_time.elapsed();
                            EMBEDDING_DURATION.observe(duration.as_secs_f64());
                            EMBEDDING_SUCCESS_TOTAL.inc();
                            EMBEDDING_PROCESSING_DELAY.observe(delay_secs);
                            info!("Generated embedding for {} {} (took {:.2}s)", file_type, hash, duration.as_secs_f64());
                        }
                        Err(e) => {
                            let duration = start_time.elapsed();
                            EMBEDDING_DURATION.observe(duration.as_secs_f64());
                            EMBEDDING_FAILURES_TOTAL.inc();
                            error!("Failed to generate embedding for {} {} (took {:.2}s): {}", file_type, hash, duration.as_secs_f64(), e);
                            // Mark permanent failures so they aren't retried
                            if crate::ai_client::is_permanent_failure(&e) {
                                let table_name = if file_type == "video" { "videos" } else { "images" };
                                if let Err(e) = crate::utils::validate_table_name(table_name) {
                                    error!("Table name validation failed for {}: {}", table_name, e);
                                    return;
                                }
                                let query = format!("UPDATE {} SET embedding_generated_at = NOW() WHERE hash = $1 AND user_id = $2", table_name);
                                let _ = client.execute(&query, &[&hash, &user_id]).await;
                            }
                        }
                    }
                    // _emb_permit automatically dropped here
                }

                if file_type == "image" && process_orientation {
                    // Acquire permit in scope to auto-release when done
                    let _orient_permit = match orient_sem_clone.acquire().await {
                        Ok(p) => p,
                        Err(e) => {
                            warn!("Orientation detection semaphore closed for {}: {}", hash, e);
                            return;
                        }
                    };
                    info!("Starting AI orientation detection for image: {}", hash);
                    let start_time = Instant::now();
                    match process_orientation_detection(&file_path, &hash, &user_id, &config_clone, &client).await {
                        Ok(()) => {
                            let duration = start_time.elapsed();
                            ORIENTATION_DURATION.observe(duration.as_secs_f64());
                            ORIENTATION_SUCCESS_TOTAL.inc();
                            ORIENTATION_PROCESSING_DELAY.observe(delay_secs);
                            info!("AI orientation detected for image {} (took {:.2}s)", hash, duration.as_secs_f64());
                        }
                        Err(e) => {
                            let duration = start_time.elapsed();
                            ORIENTATION_DURATION.observe(duration.as_secs_f64());
                            ORIENTATION_FAILURES_TOTAL.inc();
                            error!("Failed AI orientation detection for image {} (took {:.2}s): {}", hash, duration.as_secs_f64(), e);
                            // Mark as detected ONLY on permanent failures (invalid/missing
                            // input, auth errors). A transient gRPC outage must not burn the
                            // image forever — it is retried on the next cycle.
                            if crate::ai_client::is_permanent_failure(&e) {
                                if let Err(err) = client.execute(
                                    "UPDATE images SET orientation_detected_at = NOW() WHERE hash = $1 AND user_id = $2",
                                    &[&hash, &user_id]
                                ).await {
                                    error!("Failed to mark orientation detected for image {}: {}", hash, err);
                                }
                            }
                        }
                    }
                    // _orient_permit automatically dropped here
                }

                if file_type == "image" && process_faces {
                    // Acquire permit in scope to auto-release when done
                    let _face_permit = match face_sem_clone.acquire().await {
                        Ok(p) => p,
                        Err(e) => {
                            warn!("Face detection semaphore closed for {}: {}", hash, e);
                            return;
                        }
                    };
                    info!("Starting face detection for image: {}", hash);
                    let start_time = Instant::now();
                    match process_face_detection(&file_path, &hash, &user_id, db_orientation, &config_clone, &client).await {
                        Ok(count) => {
                            let duration = start_time.elapsed();
                            FACE_DETECTION_DURATION.observe(duration.as_secs_f64());
                            FACE_DETECTION_SUCCESS_TOTAL.inc();
                            FACE_DETECTION_PROCESSING_DELAY.observe(delay_secs);
                            FACES_DETECTED_TOTAL.inc_by(count as u64);
                            info!("Detected and stored {} faces for image {} (took {:.2}s)", count, hash, duration.as_secs_f64());
                            // Mark image as processed (even if 0 faces found)
                            if let Err(e) = client.execute(
                                "UPDATE images SET face_detection_completed_at = NOW() WHERE hash = $1 AND user_id = $2",
                                &[&hash, &user_id]
                            ).await {
                                error!("Failed to mark face detection complete for image {}: {}", hash, e);
                            }
                            // Mark user for batch clustering if faces were found
                            if count > 0 {
                                users_set_clone.lock().await.insert(user_id);
                            }
                        },
                        Err(e) => {
                            let duration = start_time.elapsed();
                            FACE_DETECTION_DURATION.observe(duration.as_secs_f64());
                            FACE_DETECTION_FAILURES_TOTAL.inc();
                            error!("Failed face detection for image {} (took {:.2}s): {}", hash, duration.as_secs_f64(), e);
                            // Mark as processed ONLY on permanent failures (invalid/missing
                            // input, auth errors). A transient gRPC outage must not burn
                            // the image forever — it is retried on the next cycle.
                            if crate::ai_client::is_permanent_failure(&e) {
                                if let Err(e) = client.execute(
                                    "UPDATE images SET face_detection_completed_at = NOW() WHERE hash = $1 AND user_id = $2",
                                    &[&hash, &user_id]
                                ).await {
                                    error!("Failed to mark face detection complete for image {}: {}", hash, e);
                                }
                            }
                        }
                    }
                    // _face_permit automatically dropped here
                }

                if file_type == "image" && process_quality {
                    let _quality_permit = match quality_sem_clone.acquire().await {
                        Ok(p) => p,
                        Err(e) => {
                            warn!("Quality semaphore closed for {}: {}", hash, e);
                            return;
                        }
                    };
                    info!("Starting quality scoring for image: {}", hash);
                    let file_size = std::fs::metadata(&file_path)
                        .map(|m| m.len().min(i32::MAX as u64) as i32)
                        .unwrap_or(0);
                    match tokio::fs::read(&file_path).await {
                        Ok(image_data) => {
                            match resize_image_for_ai(image_data, 384).await {
                                Ok(resized) => {
                                    match crate::services::quality::get_quality_score(&resized, &config_clone).await {
                                        Ok(q) => {
                                            let _ = client.execute(
                                                "UPDATE images SET aesthetic_score=$1, sharpness_score=$2, width=$3, height=$4, \
                                                 file_size_bytes=$5, quality_score_generated_at=NOW() \
                                                 WHERE hash=$6 AND user_id=$7",
                                                &[&q.aesthetic_score, &q.sharpness_score, &q.width, &q.height,
                                                  &file_size, &hash, &user_id],
                                            ).await;
                                            info!("Quality scored image {} (aesthetic={:.1}, sharpness={:.0})", hash, q.aesthetic_score, q.sharpness_score);
                                        }
                                        Err(e) if crate::ai_client::is_permanent_failure(&e) => {
                                            // Permanent failure (invalid image, auth error) —
                                            // mark done to avoid retrying forever.
                                            let _ = client.execute(
                                                "UPDATE images SET quality_score_generated_at=NOW() WHERE hash=$1 AND user_id=$2",
                                                &[&hash, &user_id],
                                            ).await;
                                        }
                                        Err(e) => {
                                            warn!("Quality score failed for {}: {}", hash, e);
                                        }
                                    }
                                }
                                Err(e) => warn!("Failed to resize image for quality scoring {}: {}", hash, e),
                            }
                        }
                        Err(e) => warn!("Failed to read image for quality scoring {}: {}", hash, e),
                    }
                }

                // Small breather between files to ensure system UI/background tasks stay smooth
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        })
        .await;

    // Batch clustering: cluster faces once per user after all face detection is done
    let users_to_cluster = users_with_new_faces.lock().await;
    if !users_to_cluster.is_empty() {
        info!("Clustering faces for {} users with new detections", users_to_cluster.len());
        for user_id in users_to_cluster.iter() {
            let start_time = Instant::now();
            match crate::services::face_detection::cluster_faces_for_user(user_id, &client).await {
                Ok(clustered) => {
                    let duration = start_time.elapsed();
                    FACE_CLUSTERING_DURATION.observe(duration.as_secs_f64());
                    info!("Clustered {} faces for user {} (took {:.2}s)", clustered, user_id, duration.as_secs_f64());
                }
                Err(e) => {
                    let duration = start_time.elapsed();
                    FACE_CLUSTERING_DURATION.observe(duration.as_secs_f64());
                    error!("Failed to cluster faces for user {} (took {:.2}s): {}", user_id, duration.as_secs_f64(), e);
                }
            }
        }
    }

    Ok(true)
}

async fn get_image_description(
    file_path: &PathBuf,
    hash: &str,
    file_type: &str,
    config: &Config,
) -> Result<String, String> {
    info!("Getting AI description for {} file: {}", file_type, hash);

    if file_type != "image" {
        info!("Skipping AI description for video file: {}", hash);
        return Ok(String::new());
    }

    let mut file = File::open(file_path)
        .await
        .map_err(|e| format!("Failed to open image file for description: {}", e))?;

    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)
        .await
        .map_err(|e| format!("Failed to read image file for description: {}", e))?;

    // Pre-resize to 768px max for VLM input — saves ~98% bandwidth for full-res images
    let buffer = resize_image_for_ai(buffer, 768).await?;

    let ai_client = crate::ai_client::AiClient::shared(config);

    info!("Sending gRPC describe_image request for {}", hash);
    let (description, model_used) = ai_client.describe_image(buffer, false).await?;
    info!("Successfully got AI description via gRPC model {} for {} (length: {} chars)", model_used, hash, description.len());

    Ok(description)
}

async fn generate_and_store_embedding(
    hash: &str,
    file_path: &PathBuf,
    user_id: &uuid::Uuid,
    config: &Config,
    client: &tokio_postgres::Client,
) -> Result<(), String> {
    info!("Generating embedding for image: {}", hash);

    let image_data = tokio::fs::read(file_path).await.map_err(|e| format!("Failed to read image: {}", e))?;

    // Pre-resize to 384px max for SigLIP2 input — saves ~98% bandwidth for full-res images
    let image_data = resize_image_for_ai(image_data, 384).await?;
    let embedding = crate::services::embedding::get_image_embedding(&image_data, config).await?;

    client
        .execute(
            "UPDATE images SET embedding = $1, embedding_generated_at = NOW() WHERE hash = $2 AND user_id = $3",
            &[&embedding, &hash, user_id],
        )
        .await
        .map_err(|e| format!("Failed to store embedding: {}", e))?;

    info!("Successfully stored embedding for image: {}", hash);
    Ok(())
}

async fn process_face_detection(
    file_path: &PathBuf,
    hash: &str,
    user_id: &uuid::Uuid,
    db_orientation: Option<i16>,
    config: &Config,
    client: &tokio_postgres::Client,
) -> Result<usize, String> {
    info!("Processing face detection for image: {}", hash);

    let raw_image_data = tokio::fs::read(file_path).await
        .map_err(|e| format!("Failed to read image: {}", e))?;

    // Apply orientation using the DB-stored value — avoids re-parsing EXIF from bytes.
    let orientation = db_orientation.unwrap_or(1) as u16;
    let oriented_image_data = actix_web::web::block(move || {
        crate::media_utils::orient_image_to_jpeg(&raw_image_data, orientation)
    }).await
        .map_err(|e| format!("Blocking task failed: {}", e))?
        .map_err(|e| format!("Failed to apply orientation: {}", e))?;

    // Get oriented image dimensions before resizing so we can scale bbox coords back later
    let (orig_w, orig_h) = {
        let data = oriented_image_data.clone();
        actix_web::web::block(move || {
            image::load_from_memory(&data)
                .map(|img| (img.width(), img.height()))
                .map_err(|e| format!("Failed to decode image dimensions: {}", e))
        }).await
            .map_err(|e| format!("Blocking task failed: {}", e))??
    };

    // Pre-resize to 2000px max for face detection — reduces data transfer to the AI
    // service while keeping max pixels (2000²=4,000,000) at-or-under the server's
    // MAX_IMAGE_PIXELS cap so square images near the limit aren't rejected.
    const FACE_DET_MAX_DIM: u32 = 2000;
    let resized_data = resize_image_for_ai(oriented_image_data, FACE_DET_MAX_DIM).await?;

    let faces = crate::services::face_detection::detect_faces(&resized_data, config).await?;

    if faces.is_empty() {
        info!("No faces detected in image: {}", hash);
        return Ok(0);
    }

    // Scale bbox coordinates back to original oriented image space.
    // InsightFace returned coords relative to the resized image; we need them
    // relative to the original so get_face_thumbnail can crop correctly.
    let faces = scale_bboxes_to_original(faces, orig_w, orig_h, FACE_DET_MAX_DIM);

    crate::services::face_detection::store_faces(hash, user_id, faces, client).await
}

/// Minimum softmax confidence for an AI orientation prediction to be trusted.
/// The BEiT classifier is photo-trained — on screenshots/graphics its top-1 score is
/// near-random (~0.4), so gating prevents wrongly rotating non-photos.
const ORIENTATION_MIN_CONFIDENCE: f32 = 0.5;

/// Detect the rotation of an image that carries no EXIF orientation, using the AI
/// image-classifier fallback, and store the equivalent EXIF orientation value
/// (1/3/6/8) so `get_image`/thumbnail generation right the photo. Race-safe: the
/// UPDATE only writes when `orientation IS NULL` (ingest may have set it meanwhile).
async fn process_orientation_detection(
    file_path: &PathBuf,
    hash: &str,
    user_id: &uuid::Uuid,
    config: &Config,
    client: &tokio_postgres::Client,
) -> Result<(), String> {
    let image_data = tokio::fs::read(file_path).await
        .map_err(|e| format!("Failed to read image: {}", e))?;

    // Pre-resize to the classifier's 384px input — also keeps the gRPC payload small.
    let resized = resize_image_for_ai(image_data, 384).await?;

    let ai_client = crate::ai_client::AiClient::shared(config);
    let detection = ai_client.detect_orientation(resized).await?;

    let stored = match detection.orientation_value {
        v @ (1 | 3 | 6 | 8) if detection.confidence >= ORIENTATION_MIN_CONFIDENCE => Some(v as i16),
        _ => None,
    };

    // Mark the attempt complete regardless (below-min-confidence results are treated
    // as "checked, nothing to fix") so EXIF-less images aren't re-sent every cycle.
    if let Some(o) = stored {
        client.execute(
            "UPDATE images SET orientation = $1, orientation_detected_at = NOW() \
             WHERE hash = $2 AND user_id = $3 AND orientation IS NULL",
            &[&o, &hash, user_id],
        ).await
            .map_err(|e| format!("Failed to store orientation: {}", e))?;
        info!("Stored AI-detected orientation {} for image {} (label={}, conf={:.3})",
              o, hash, detection.label, detection.confidence);
    } else {
        client.execute(
            "UPDATE images SET orientation_detected_at = NOW() WHERE hash = $1 AND user_id = $2",
            &[&hash, user_id],
        ).await
            .map_err(|e| format!("Failed to mark orientation detection attempt: {}", e))?;
        info!("Orientation detection inconclusive for {} (label={}, conf={:.3}) — leaving unrotated",
              hash, detection.label, detection.confidence);
    }

    Ok(())
}

/// Scale face bbox coordinates from detection-image space back to original image space.
/// `resize_image_for_ai` fits the image within `max_dim x max_dim` maintaining aspect ratio,
/// so the scale factor is `max(orig_w, orig_h) / max_dim` when downscaling occurred.
fn scale_bboxes_to_original(
    faces: Vec<(Vec<i32>, pgvector::Vector, f32)>,
    orig_w: u32,
    orig_h: u32,
    max_dim: u32,
) -> Vec<(Vec<i32>, pgvector::Vector, f32)> {
    let max_orig = orig_w.max(orig_h);
    if max_orig <= max_dim {
        return faces; // Image was not downscaled, coords are already correct
    }
    let scale = max_orig as f64 / max_dim as f64;
    faces.into_iter().map(|(bbox, embedding, confidence)| {
        let scaled = bbox.iter().map(|&v| (v as f64 * scale).round() as i32).collect();
        (scaled, embedding, confidence)
    }).collect()
}

async fn generate_and_store_video_embedding(
    hash: &str,
    file_path: &std::path::Path,
    user_id: &uuid::Uuid,
    config: &Config,
    client: &tokio_postgres::Client,
) -> Result<(), String> {
    info!("Generating video keyframe embeddings for video: {}", hash);

    let keyframes = crate::media_utils::extract_video_keyframes(file_path, 5.0, 10).await?;
    if keyframes.is_empty() {
        warn!("No keyframes extracted from video: {}", hash);
        client
            .execute(
                "UPDATE videos SET embedding_generated_at = NOW() WHERE hash = $1 AND user_id = $2",
                &[&hash, user_id],
            )
            .await
            .map_err(|e| format!("Failed to mark video embedding complete: {}", e))?;
        return Ok(());
    }

    let ai_client = crate::ai_client::AiClient::shared(config);

    let mut keyframe_embeddings = Vec::new();
    for (timestamp, raw_bytes) in keyframes {
        let resized = resize_image_for_ai(raw_bytes, 384).await?;
        match ai_client.embed_image(resized).await {
            Ok(vec) if vec.len() == 1152 => {
                let vector = pgvector::Vector::from(vec.clone());
                if let Err(e) = client
                    .execute(
                        "INSERT INTO video_keyframes (video_hash, user_id, timestamp_secs, embedding)
                         VALUES ($1, $2, $3, $4)
                         ON CONFLICT (user_id, video_hash, timestamp_secs) DO UPDATE SET embedding = EXCLUDED.embedding",
                        &[&hash, user_id, &timestamp, &vector],
                    )
                    .await
                {
                    warn!("Failed to store keyframe for video {} at {:.1}s: {}", hash, timestamp, e);
                }
                keyframe_embeddings.push(vec);
            }
            Ok(vec) => warn!("Invalid video keyframe embedding dim: {}", vec.len()),
            Err(e) => warn!("Keyframe embedding error for video {} at {:.1}s: {}", hash, timestamp, e),
        }
    }

    if keyframe_embeddings.is_empty() {
        return Err(format!("Failed to generate keyframe embeddings for video {}", hash));
    }

    // Compute mean (centroid) vector across keyframes
    let dim = 1152;
    let mut centroid = vec![0.0f32; dim];
    let count = keyframe_embeddings.len() as f32;

    for emb in &keyframe_embeddings {
        for i in 0..dim {
            centroid[i] += emb[i] / count;
        }
    }

    // Normalize centroid
    let norm = centroid.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut centroid {
            *x /= norm;
        }
    }

    let centroid_vector = pgvector::Vector::from(centroid);
    client
        .execute(
            "UPDATE videos SET embedding = $1, embedding_generated_at = NOW() WHERE hash = $2 AND user_id = $3",
            &[&centroid_vector, &hash, user_id],
        )
        .await
        .map_err(|e| format!("Failed to store video embedding: {}", e))?;

    info!("Successfully stored {} keyframes and centroid embedding for video {}", keyframe_embeddings.len(), hash);
    Ok(())
}

async fn update_status_metrics(client: &tokio_postgres::Client) -> Result<(), tokio_postgres::Error> {
    // Thumbnail coverage is derived from the DB (has_thumbnail flag), not from the
    // thumbnail_success_total event counter, which resets on every server restart and
    // would otherwise massively under-report coverage.
    let row = client.query_one(
        "WITH img AS (
            SELECT COUNT(*) AS total,
                   COUNT(embedding) AS with_embedding,
                   COUNT(description) AS with_description,
                   COUNT(face_detection_completed_at) AS face_processed,
                   COUNT(*) FILTER (WHERE has_thumbnail = true) AS with_thumbnail
            FROM images
         ), vid AS (
            SELECT COUNT(*) AS total,
                   COUNT(*) FILTER (WHERE has_thumbnail = true) AS with_thumbnail
            FROM videos
         )
         SELECT img.total, img.with_embedding, img.with_description, img.face_processed,
                img.with_thumbnail + vid.with_thumbnail,
                img.total + vid.total
         FROM img, vid",
        &[]
    ).await?;

    let total: i64 = row.get(0);
    let with_embedding: i64 = row.get(1);
    let with_description: i64 = row.get(2);
    let face_processed: i64 = row.get(3);
    let thumbnail_count: i64 = row.get(4);
    let total_media: i64 = row.get(5);

    TOTAL_IMAGES.set(total);
    IMAGES_WITH_EMBEDDING.set(with_embedding);
    IMAGES_WITH_DESCRIPTION.set(with_description);
    IMAGES_FACE_PROCESSED.set(face_processed);

    THUMBNAIL_COUNT.set(thumbnail_count);
    if total_media > 0 {
        THUMBNAIL_COVERAGE.set(thumbnail_count as f64 / total_media as f64);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_faces(bboxes: &[[i32; 4]]) -> Vec<(Vec<i32>, pgvector::Vector, f32)> {
        bboxes.iter().map(|b| {
            (b.to_vec(), pgvector::Vector::from(vec![0.0f32; 512]), 0.99)
        }).collect()
    }

    #[test]
    fn no_scale_when_image_fits_in_max_dim() {
        // 1000x800 image — no resize, coords unchanged
        let faces = dummy_faces(&[[100, 50, 200, 200]]);
        let result = scale_bboxes_to_original(faces, 1000, 800, 2048);
        assert_eq!(result[0].0, vec![100, 50, 200, 200]);
    }

    #[test]
    fn no_scale_when_image_exactly_max_dim() {
        let faces = dummy_faces(&[[100, 50, 200, 200]]);
        let result = scale_bboxes_to_original(faces, 2048, 1536, 2048);
        assert_eq!(result[0].0, vec![100, 50, 200, 200]);
    }

    #[test]
    fn scales_up_for_landscape_image() {
        // 4096x3072 landscape → downscaled to 2048x1536 (scale = 2.0)
        // Face at [200, 100, 300, 300] in detection space → [400, 200, 600, 600] in original
        let faces = dummy_faces(&[[200, 100, 300, 300]]);
        let result = scale_bboxes_to_original(faces, 4096, 3072, 2048);
        assert_eq!(result[0].0, vec![400, 200, 600, 600]);
    }

    #[test]
    fn scales_up_for_portrait_image() {
        // 3000x4000 portrait → downscaled to 1536x2048 (scale = 4000/2048 ≈ 1.953)
        // Face at [100, 200, 150, 150] → scaled
        let faces = dummy_faces(&[[100, 200, 150, 150]]);
        let result = scale_bboxes_to_original(faces, 3000, 4000, 2048);
        let scale = 4000.0f64 / 2048.0;
        let expected: Vec<i32> = [100, 200, 150, 150].iter()
            .map(|&v| (v as f64 * scale).round() as i32)
            .collect();
        assert_eq!(result[0].0, expected);
    }

    #[test]
    fn handles_multiple_faces() {
        let faces = dummy_faces(&[[100, 100, 200, 200], [500, 400, 300, 300]]);
        let result = scale_bboxes_to_original(faces, 4096, 3072, 2048);
        assert_eq!(result[0].0, vec![200, 200, 400, 400]);
        assert_eq!(result[1].0, vec![1000, 800, 600, 600]);
    }
}
