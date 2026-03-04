use super::utils::{get_load_average, get_cpu_count, calculate_worker_concurrency};
use crate::config::Config;
use crate::db::MainDbPool;
use crate::metrics::{
    AI_DESCRIPTION_DURATION, AI_DESCRIPTION_SUCCESS_TOTAL, AI_DESCRIPTION_FAILURES_TOTAL,
    EMBEDDING_DURATION, EMBEDDING_SUCCESS_TOTAL, EMBEDDING_FAILURES_TOTAL,
    FACE_DETECTION_DURATION, FACE_DETECTION_SUCCESS_TOTAL, FACE_DETECTION_FAILURES_TOTAL,
    FACES_DETECTED_TOTAL, FACE_CLUSTERING_DURATION,
    AI_DESCRIPTION_PROCESSING_DELAY, EMBEDDING_PROCESSING_DELAY, FACE_DETECTION_PROCESSING_DELAY,
    TOTAL_IMAGES, IMAGES_WITH_EMBEDDING, IMAGES_WITH_DESCRIPTION, IMAGES_FACE_PROCESSED,
};
use actix_web::web;
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use once_cell::sync::Lazy;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::sync::Semaphore;
use tokio::time::Duration;
use base64::{Engine as _, engine::general_purpose};
use futures::stream::{self, StreamExt};
use chrono::{DateTime, Utc};

#[derive(Serialize)]
struct VlmRequest {
    image: String,
}

#[derive(Deserialize)]
struct VlmResponse {
    description: String,
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

pub async fn start_ai_worker(pool: web::Data<MainDbPool>, config: web::Data<Config>) {
    info!("AI worker started.");
    
    // Adaptive strategy:
    // - Active: 5s (Process heavy backlog slightly faster than default)
    // - Idle: Backoff up to 30s
    super::utils::run_worker_loop(
        "AI Worker",
        Duration::from_secs(5),
        Duration::from_secs(30),
        || process_files(pool.clone(), config.clone())
    ).await;
}

async fn process_files(pool: web::Data<MainDbPool>, config: web::Data<Config>) -> Result<bool, String> {
    // Periodically update overall library status metrics
    static LAST_STATUS_UPDATE: Lazy<std::sync::Mutex<Option<Instant>>> = Lazy::new(|| std::sync::Mutex::new(None));
    
    let should_update = {
        let mut last_update = LAST_STATUS_UPDATE.lock().unwrap();
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

    if !enable_ai_descriptions && !enable_embeddings && !enable_face_detection {
        info!("AI descriptions, embeddings, and face detection are all disabled, skipping this cycle.");
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
    let description_concurrency = limits.description;

    // We use a total task limit to prevent fetching too many tasks at once
    let total_batch_limit = (embedding_concurrency + face_concurrency + description_concurrency) * 2;
    let mut all_tasks_to_process = std::collections::HashMap::new();

    // --- STRICT PRIORITY FETCHING ---
    
    // 1. HIGH PRIORITY: Embeddings
    if enable_embeddings {
        let embedding_rows = client
            .query(
                "SELECT hash, ext, name, deviceid, 'image' as file_type, created_at FROM images
                 WHERE verification_status = 1 AND embedding IS NULL AND embedding_generated_at IS NULL
                 LIMIT $1",
                &[&(total_batch_limit as i64)],
            )
            .await
            .map_err(|e| format!("Failed to query for AI embedding tasks: {}", e))?;
        
        for row in embedding_rows {
            let hash: String = row.get(0);
            let ext: String = row.get(1);
            let name: String = row.get(2);
            let deviceid: String = row.get(3);
            let file_type: String = row.get(4);
            let created_at: DateTime<Utc> = row.get(5);
            all_tasks_to_process.insert(hash.clone(), (hash, ext, name, deviceid, file_type, None, false, true, false, false, created_at));
        }
    }

    // 2. MEDIUM PRIORITY: Face Detection
    // Only fetch if we have room in our batch limit
    if enable_face_detection && all_tasks_to_process.len() < total_batch_limit {
        let room_left = total_batch_limit - all_tasks_to_process.len();
        let face_detection_rows = client
            .query(
                "SELECT i.hash, i.ext, i.name, i.deviceid, i.user_id, 'image' as file_type, i.created_at
                 FROM images i
                 WHERE i.verification_status = 1
                   AND i.face_detection_completed_at IS NULL
                 LIMIT $1",
                &[&(room_left as i64)],
            )
            .await
            .map_err(|e| format!("Failed to query for face detection tasks: {}", e))?;

        for row in face_detection_rows {
            let hash: String = row.get(0);
            let ext: String = row.get(1);
            let name: String = row.get(2);
            let deviceid: String = row.get(3);
            let user_id: uuid::Uuid = row.get(4);
            let file_type: String = row.get(5);
            let created_at: DateTime<Utc> = row.get(6);
            
            all_tasks_to_process.entry(hash.clone())
                .and_modify(|e| {
                    e.5 = Some(user_id);
                    e.8 = true;
                })
                .or_insert((hash, ext, name, deviceid, file_type, Some(user_id), false, false, true, false, created_at));
        }
    }

    // 3. LOW PRIORITY: Descriptions
    // Only fetch if we still have room AND GPU is not at absolute capacity (> 95%)
    if enable_ai_descriptions && all_tasks_to_process.len() < total_batch_limit && gpu_load < 95 {
        let room_left = total_batch_limit - all_tasks_to_process.len();
        let description_rows = client
            .query(
                "SELECT hash, ext, name, deviceid, 'image' as file_type, created_at FROM images
                 WHERE verification_status = 1 AND description IS NULL
                 LIMIT $1",
                &[&(room_left as i64)],
            )
            .await
            .map_err(|e| format!("Failed to query for AI description tasks: {}", e))?;

        for row in description_rows {
            let hash: String = row.get(0);
            let ext: String = row.get(1);
            let name: String = row.get(2);
            let deviceid: String = row.get(3);
            let file_type: String = row.get(4);
            let created_at: DateTime<Utc> = row.get(5);
            
            all_tasks_to_process.entry(hash.clone())
                .and_modify(|e| e.6 = true)
                .or_insert((hash, ext, name, deviceid, file_type, None, true, false, false, false, created_at));
        }
    }

    // 4. LOWEST PRIORITY: Quality scoring
    // Only for images that already have embeddings; skip if system is busy with higher-priority tasks
    if all_tasks_to_process.len() < total_batch_limit {
        let room_left = total_batch_limit - all_tasks_to_process.len();
        let quality_rows = client
            .query(
                "SELECT hash, ext, name, deviceid, 'image' as file_type, created_at FROM images
                 WHERE verification_status = 1
                   AND embedding IS NOT NULL
                   AND quality_score_generated_at IS NULL
                 LIMIT $1",
                &[&(room_left as i64)],
            )
            .await
            .map_err(|e| format!("Failed to query for quality tasks: {}", e))?;

        for row in quality_rows {
            let hash: String = row.get(0);
            let ext: String = row.get(1);
            let name: String = row.get(2);
            let deviceid: String = row.get(3);
            let file_type: String = row.get(4);
            let created_at: DateTime<Utc> = row.get(5);

            all_tasks_to_process.entry(hash.clone())
                .and_modify(|e| e.9 = true)
                .or_insert((hash, ext, name, deviceid, file_type, None, false, false, false, true, created_at));
        }
    }

    if all_tasks_to_process.is_empty() {
        return Ok(false);
    }

    let tasks_to_process: Vec<_> = all_tasks_to_process.into_values().collect();
    let total_files = tasks_to_process.len();

    // Concurrency limit for the parallel stream
    let quality_concurrency = description_concurrency; // Same as description: lowest priority
    let concurrent_limit = (description_concurrency + embedding_concurrency + face_concurrency + quality_concurrency) * 2;

    info!("AI cycle: Processing {} files [CPU Load: {:.1}, GPU Load: {}%]",
          total_files, load_average, gpu_load);

    // Semaphores enforce priority through weighted concurrency:
    let description_semaphore = Arc::new(Semaphore::new(description_concurrency));
    let embedding_semaphore = Arc::new(Semaphore::new(embedding_concurrency));
    let face_semaphore = Arc::new(Semaphore::new(face_concurrency));
    let quality_semaphore = Arc::new(Semaphore::new(quality_concurrency));

    // Track users that had faces detected for batch clustering at the end
    let users_with_new_faces = Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new()));
    let users_with_new_faces_clone = users_with_new_faces.clone();

    stream::iter(tasks_to_process)
        .for_each_concurrent(concurrent_limit, move |(hash, ext, _name, deviceid, file_type, user_id, process_description, process_embedding, process_faces, process_quality, created_at)| {
            let file_dir = if file_type == "image" { config.get_images_dir().to_string() } else { config.get_videos_dir().to_string() };
            let sub_dir_path = super::utils::get_subdirectory_path(&file_dir, &hash);
            let file_path = sub_dir_path.join(format!("{}.{}", hash, ext));

            let pool_clone = pool.clone();
            let config_clone = config.clone();
            let desc_sem_clone = description_semaphore.clone();
            let emb_sem_clone = embedding_semaphore.clone();
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
                    let _desc_permit = desc_sem_clone.acquire().await.unwrap();
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
                            let query = format!("UPDATE {} SET description = $1 WHERE hash = $2 AND deviceid = $3", table_name);
                            if let Err(e) = client.execute(&query, &[&desc, &hash, &deviceid]).await {
                                error!("Failed to update description for {} {}: {}", file_type, hash, e);
                            }
                        }
                        Ok(_) => {}
                        Err(e) => {
                            let duration = start_time.elapsed();
                            AI_DESCRIPTION_DURATION.observe(duration.as_secs_f64());
                            AI_DESCRIPTION_FAILURES_TOTAL.inc();
                            error!("Failed to get AI description for {} {} (took {:.2}s): {}", file_type, hash, duration.as_secs_f64(), e);
                            // Mark permanent failures (400 Bad Request) so they aren't retried
                            if e.contains("400 Bad Request") {
                                let table_name = if file_type == "image" { "images" } else { "videos" };
                                let query = format!("UPDATE {} SET description = $1 WHERE hash = $2 AND deviceid = $3", table_name);
                                let _ = client.execute(&query, &[&"[skipped]", &hash, &deviceid]).await;
                            }
                        }
                    }
                    // _ai_permit automatically dropped here
                }

                if file_type == "image" && process_embedding {
                    // Acquire permit in scope to auto-release when done
                    let _emb_permit = emb_sem_clone.acquire().await.unwrap();
                    info!("Starting embedding generation for image: {}", hash);
                    let start_time = Instant::now();
                    match generate_and_store_embedding(&hash, &file_path, &deviceid, &config_clone, &client).await {
                        Ok(_) => {
                            let duration = start_time.elapsed();
                            EMBEDDING_DURATION.observe(duration.as_secs_f64());
                            EMBEDDING_SUCCESS_TOTAL.inc();
                            EMBEDDING_PROCESSING_DELAY.observe(delay_secs);
                            info!("Generated embedding for image {} (took {:.2}s)", hash, duration.as_secs_f64());
                        }
                        Err(e) => {
                            let duration = start_time.elapsed();
                            EMBEDDING_DURATION.observe(duration.as_secs_f64());
                            EMBEDDING_FAILURES_TOTAL.inc();
                            error!("Failed to generate embedding for image {} (took {:.2}s): {}", hash, duration.as_secs_f64(), e);
                            // Mark permanent failures (400 Bad Request) so they aren't retried
                            if e.contains("400 Bad Request") {
                                let _ = client.execute(
                                    "UPDATE images SET embedding_generated_at = NOW() WHERE hash = $1 AND deviceid = $2",
                                    &[&hash, &deviceid],
                                ).await;
                            }
                        }
                    }
                    // _emb_permit automatically dropped here
                }

                if file_type == "image" && process_faces {
                    if let Some(uid) = user_id {
                        // Acquire permit in scope to auto-release when done
                        let _face_permit = face_sem_clone.acquire().await.unwrap();
                        info!("Starting face detection for image: {}", hash);
                        let start_time = Instant::now();
                        match process_face_detection(&file_path, &hash, &deviceid, &uid, &config_clone, &client).await {
                            Ok(count) => {
                                let duration = start_time.elapsed();
                                FACE_DETECTION_DURATION.observe(duration.as_secs_f64());
                                FACE_DETECTION_SUCCESS_TOTAL.inc();
                                FACE_DETECTION_PROCESSING_DELAY.observe(delay_secs);
                                FACES_DETECTED_TOTAL.inc_by(count as u64);
                                info!("Detected and stored {} faces for image {} (took {:.2}s)", count, hash, duration.as_secs_f64());
                                // Mark image as processed (even if 0 faces found)
                                if let Err(e) = client.execute(
                                    "UPDATE images SET face_detection_completed_at = NOW() WHERE hash = $1 AND deviceid = $2",
                                    &[&hash, &deviceid]
                                ).await {
                                    error!("Failed to mark face detection complete for image {}: {}", hash, e);
                                }
                                // Mark user for batch clustering if faces were found
                                if count > 0 {
                                    users_set_clone.lock().await.insert(uid);
                                }
                            },
                            Err(e) => {
                                let duration = start_time.elapsed();
                                FACE_DETECTION_DURATION.observe(duration.as_secs_f64());
                                FACE_DETECTION_FAILURES_TOTAL.inc();
                                error!("Failed face detection for image {} (took {:.2}s): {}", hash, duration.as_secs_f64(), e);
                                // Still mark as processed to avoid retry loops
                                if let Err(e) = client.execute(
                                    "UPDATE images SET face_detection_completed_at = NOW() WHERE hash = $1 AND deviceid = $2",
                                    &[&hash, &deviceid]
                                ).await {
                                    error!("Failed to mark face detection complete for image {}: {}", hash, e);
                                }
                            }
                        }
                        // _face_permit automatically dropped here
                    }
                }

                if file_type == "image" && process_quality {
                    let _quality_permit = quality_sem_clone.acquire().await.unwrap();
                    info!("Starting quality scoring for image: {}", hash);
                    let file_size = std::fs::metadata(&file_path)
                        .map(|m| m.len() as i32)
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
                                                 WHERE hash=$6 AND deviceid=$7",
                                                &[&q.aesthetic_score, &q.sharpness_score, &q.width, &q.height,
                                                  &file_size, &hash, &deviceid],
                                            ).await;
                                            info!("Quality scored image {} (aesthetic={:.1}, sharpness={:.0})", hash, q.aesthetic_score, q.sharpness_score);
                                        }
                                        Err(e) if e.contains("400") => {
                                            // Permanent failure — mark done to avoid retry
                                            let _ = client.execute(
                                                "UPDATE images SET quality_score_generated_at=NOW() WHERE hash=$1 AND deviceid=$2",
                                                &[&hash, &deviceid],
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
    let base64_image = general_purpose::STANDARD.encode(&buffer);

    let request = VlmRequest {
        image: base64_image,
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let ai_url = format!("{}/describe", config.embedding_service_url); 
    
    info!("Sending request to AI service at: {}", ai_url);
    let response = client
        .post(&ai_url)
        .json(&request)
        .send()
        .await
        .map_err(|e| format!("Failed to send request to AI service: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_default();
        return Err(format!("AI API returned error: {} - {}", status, error_text));
    }

    let response_text = response.text().await.map_err(|e| format!("Failed to get response text: {}", e))?;
    
    let vlm_response: VlmResponse =
        serde_json::from_str(&response_text).map_err(|e| format!("Failed to parse AI response: {}", e))?;

    let description = vlm_response.description;
    info!("Successfully got AI description for {} (length: {} chars)", hash, description.len());

    Ok(description)
}

async fn generate_and_store_embedding(
    hash: &str,
    file_path: &PathBuf,
    deviceid: &str,
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
            "UPDATE images SET embedding = $1, embedding_generated_at = NOW() WHERE hash = $2 AND deviceid = $3",
            &[&embedding, &hash, &deviceid],
        )
        .await
        .map_err(|e| format!("Failed to store embedding: {}", e))?;

    info!("Successfully stored embedding for image: {}", hash);
    Ok(())
}

/// Apply EXIF orientation to image bytes and return correctly oriented JPEG bytes
fn apply_exif_orientation(image_data: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Cursor;

    // Decode the image
    let mut img = image::load_from_memory(image_data)
        .map_err(|e| format!("Failed to decode image: {}", e))?;

    // Read EXIF orientation from the original bytes
    let cursor = Cursor::new(image_data);
    let mut bufreader = std::io::BufReader::new(cursor);
    let exifreader = kamadak_exif::Reader::new();

    if let Ok(exif) = exifreader.read_from_container(&mut bufreader) {
        if let Some(field) = exif.get_field(kamadak_exif::Tag::Orientation, kamadak_exif::In::PRIMARY) {
            if let kamadak_exif::Value::Short(ref v) = field.value {
                if let Some(&orientation) = v.first() {
                    match orientation {
                        1 => {}, // Normal - no change needed
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

    // Re-encode as JPEG
    let mut output = Cursor::new(Vec::new());
    img.write_to(&mut output, image::ImageOutputFormat::Jpeg(90))
        .map_err(|e| format!("Failed to encode oriented image: {}", e))?;

    Ok(output.into_inner())
}

async fn process_face_detection(
    file_path: &PathBuf,
    hash: &str,
    deviceid: &str,
    user_id: &uuid::Uuid,
    config: &Config,
    client: &tokio_postgres::Client,
) -> Result<usize, String> {
    info!("Processing face detection for image: {}", hash);

    let raw_image_data = tokio::fs::read(file_path).await
        .map_err(|e| format!("Failed to read image: {}", e))?;

    // Apply EXIF orientation before face detection
    // This ensures faces are correctly oriented for detection and bbox coordinates match the oriented image
    let oriented_image_data = actix_web::web::block(move || {
        apply_exif_orientation(&raw_image_data)
    }).await
        .map_err(|e| format!("Blocking task failed: {}", e))?
        .map_err(|e| format!("Failed to apply orientation: {}", e))?;

    // Pre-resize to 2048px max for face detection — reduces data transfer to AI service
    let oriented_image_data = resize_image_for_ai(oriented_image_data, 2048).await?;

    let faces = crate::services::face_detection::detect_faces(&oriented_image_data, config).await?;

    if faces.is_empty() {
        info!("No faces detected in image: {}", hash);
        return Ok(0);
    }

    crate::services::face_detection::store_faces(hash, deviceid, user_id, faces, client).await
}

async fn update_status_metrics(client: &tokio_postgres::Client) -> Result<(), tokio_postgres::Error> {
    let row = client.query_one(
        "SELECT 
            COUNT(*) as total, 
            COUNT(embedding) as with_embedding, 
            COUNT(description) as with_description,
            COUNT(face_detection_completed_at) as face_processed
        FROM images",
        &[]
    ).await?;

    let total: i64 = row.get(0);
    let with_embedding: i64 = row.get(1);
    let with_description: i64 = row.get(2);
    let face_processed: i64 = row.get(3);

    TOTAL_IMAGES.set(total);
    IMAGES_WITH_EMBEDDING.set(with_embedding);
    IMAGES_WITH_DESCRIPTION.set(with_description);
    IMAGES_FACE_PROCESSED.set(face_processed);

    Ok(())
}
