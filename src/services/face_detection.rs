use log::{error, info};
use serde::{Deserialize, Serialize};
use pgvector::Vector;
use crate::config::Config;

#[derive(Serialize)]
struct FaceDetectRequest {
    image: String,  // base64 encoded
}

#[derive(Deserialize, Debug)]
struct FaceDetectResponse {
    faces: Vec<DetectedFace>,
    count: usize,
}

#[derive(Deserialize, Debug)]
struct DetectedFace {
    bbox: [i32; 4],  // [x, y, width, height]
    embedding: Vec<f32>,
    confidence: f32,
}

/// Detect faces in an image and return bounding boxes with embeddings
pub async fn detect_faces(
    image_data: &[u8],
    config: &Config,
) -> Result<Vec<(Vec<i32>, Vector, f32)>, String> {
    info!("Requesting face detection from face service at: {}", config.face_service_url);

    let base64_image = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, image_data);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let url = format!("{}/detect", config.face_service_url);
    let request = FaceDetectRequest { image: base64_image };

    let response = client
        .post(&url)
        .json(&request)
        .send()
        .await
        .map_err(|e| format!("Failed to send request to face service: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Face service returned error: {}", response.status()));
    }

    let data: FaceDetectResponse = response.json().await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    info!("Detected {} faces", data.count);

    // Convert to internal format: (bbox, embedding, confidence)
    let results: Vec<(Vec<i32>, Vector, f32)> = data.faces
        .into_iter()
        .filter(|face| face.embedding.len() == 512)  // Validate dimension (InsightFace)
        .map(|face| {
            let bbox = face.bbox.to_vec();
            let embedding = Vector::from(face.embedding);
            (bbox, embedding, face.confidence)
        })
        .collect();

    Ok(results)
}

/// Store detected faces in the database
pub async fn store_faces(
    hash: &str,
    user_id: &uuid::Uuid,
    faces: Vec<(Vec<i32>, Vector, f32)>,
    client: &tokio_postgres::Client,
) -> Result<usize, String> {
    let mut stored_count = 0;

    for (bbox, embedding, confidence) in faces {
        if bbox.len() != 4 {
            continue;
        }

        let result = client
            .execute(
                "INSERT INTO faces (image_hash, image_user_id, user_id, bbox_x, bbox_y, bbox_width, bbox_height, embedding, confidence)
                 VALUES ($1, $2, $2, $3, $4, $5, $6, $7, $8)
                 ON CONFLICT DO NOTHING",
                &[&hash, &user_id, &bbox[0], &bbox[1], &bbox[2], &bbox[3], &embedding, &confidence],
            )
            .await
            .map_err(|e| format!("Failed to store face: {}", e))?;

        if result > 0 {
            stored_count += 1;
        }
    }

    info!("Stored {} faces for image {}", stored_count, hash);
    Ok(stored_count)
}

/// Cluster faces for a user using threshold-based approach
/// Finds similar existing persons or creates new person clusters
pub async fn cluster_faces_for_user(
    user_id: &uuid::Uuid,
    client: &tokio_postgres::Client,
) -> Result<usize, String> {
    info!("Clustering faces for user: {}", user_id);

    // Simple threshold-based clustering
    // For each unclustered face, find similar faces and create/assign to cluster
    // Higher threshold = more strict matching (fewer false positives)
    // For group photos with multiple people, need very high threshold to avoid merging
    // Industry standard for verification: 0.90+ (we need strict matching for groups)
    let threshold: f64 = 0.60;  // Lowered to 60% for InsightFace 512-dim embeddings

    // Get unclustered faces ordered by detection time
    let rows = client
        .query(
            "SELECT id, embedding FROM faces
             WHERE user_id = $1 AND person_id IS NULL
             ORDER BY detected_at",
            &[user_id],
        )
        .await
        .map_err(|e| format!("Failed to query faces: {}", e))?;

    if rows.is_empty() {
        info!("No unclustered faces found for user {}", user_id);
        return Ok(0);
    }

    info!("Found {} unclustered faces for user {}", rows.len(), user_id);
    let mut clustered = 0;

    for row in rows {
        let face_id: i64 = row.get(0);
        let embedding: Vector = row.get(1);

        // Find similar existing person by searching real face embeddings directly.
        // This handles merged persons correctly — no synthetic average that can drift.
        let similar_person = client
            .query_opt(
                "SELECT person_id FROM faces
                 WHERE user_id = $1 AND person_id IS NOT NULL
                 AND (1 - (embedding <=> $2)) > $3
                 ORDER BY embedding <=> $2
                 LIMIT 1",
                &[user_id, &embedding, &threshold],
            )
            .await
            .map_err(|e| format!("Failed to find similar person: {}", e))?;

        let person_id = if let Some(row) = similar_person {
            // Assign to existing person
            row.get::<_, i64>(0)
        } else {
            // Create new person cluster
            let row = client
                .query_one(
                    "INSERT INTO persons (user_id, representative_face_id, face_count)
                     VALUES ($1, $2, 1)
                     RETURNING id",
                    &[user_id, &face_id],
                )
                .await
                .map_err(|e| format!("Failed to create person: {}", e))?;

            info!("Created new person cluster for user {}", user_id);
            row.get(0)
        };

        // Assign face to person
        client
            .execute(
                "UPDATE faces SET person_id = $1 WHERE id = $2",
                &[&person_id, &face_id],
            )
            .await
            .map_err(|e| format!("Failed to assign face: {}", e))?;

        // Update person stats, representative embedding, and pick the face
        // closest to the centroid as the thumbnail (most "typical" face)
        client
            .execute(
                "UPDATE persons SET
                    face_count = (SELECT COUNT(*) FROM faces WHERE person_id = $1),
                    representative_embedding = (
                        SELECT AVG(embedding) FROM faces WHERE person_id = $1
                    ),
                    representative_face_id = (
                        SELECT id FROM faces WHERE person_id = $1
                        ORDER BY embedding <=> (SELECT AVG(embedding) FROM faces WHERE person_id = $1)
                        LIMIT 1
                    ),
                    updated_at = NOW()
                 WHERE id = $1",
                &[&person_id],
            )
            .await
            .map_err(|e| format!("Failed to update person: {}", e))?;

        clustered += 1;
    }

    info!("Clustered {} faces for user {}", clustered, user_id);
    Ok(clustered)
}

/// Run clustering for all users with unclustered faces
pub async fn cluster_all_users(
    client: &tokio_postgres::Client,
) -> Result<usize, String> {
    // Get all users with unclustered faces
    let rows = client
        .query(
            "SELECT DISTINCT user_id FROM faces WHERE person_id IS NULL",
            &[],
        )
        .await
        .map_err(|e| format!("Failed to query users: {}", e))?;

    let mut total_clustered = 0;

    for row in rows {
        let user_id: uuid::Uuid = row.get(0);
        match cluster_faces_for_user(&user_id, client).await {
            Ok(count) => total_clustered += count,
            Err(e) => error!("Failed to cluster faces for user {}: {}", user_id, e),
        }
    }

    Ok(total_clustered)
}
