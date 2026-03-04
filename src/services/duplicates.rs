use actix_web::{get, web, HttpRequest, HttpResponse};
use log::error;
use serde::{Serialize, Deserialize};
use utoipa::{ToSchema, IntoParams};
use std::collections::HashMap;
use chrono::{DateTime, Utc};

use crate::config::Config;
use crate::utils;
use crate::db::MainDbPool;

#[derive(Serialize, ToSchema, Clone)]
pub struct DuplicateImage {
    pub hash: String,
    pub deviceid: String,
    pub name: String,
    pub created_at: String,
    pub thumbnail_url: String,
    pub aesthetic_score: Option<f32>,
    pub sharpness_score: Option<f32>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub file_size_bytes: Option<i32>,
}

#[derive(Serialize, ToSchema)]
pub struct DuplicateGroup {
    pub similarity: f32,
    pub images: Vec<DuplicateImage>,
}

#[derive(Serialize, ToSchema)]
pub struct DuplicatesResponse {
    pub groups: Vec<DuplicateGroup>,
    pub total_groups: usize,
}

#[derive(Deserialize, IntoParams)]
pub struct DuplicatesQuery {
    /// Cosine similarity threshold (0.80–1.0). Default 0.95.
    #[serde(default = "default_threshold")]
    pub threshold: f64,
}

fn default_threshold() -> f64 {
    0.95
}

struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        UnionFind {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }

    fn union(&mut self, x: usize, y: usize) {
        let rx = self.find(x);
        let ry = self.find(y);
        if rx == ry {
            return;
        }
        if self.rank[rx] < self.rank[ry] {
            self.parent[rx] = ry;
        } else if self.rank[rx] > self.rank[ry] {
            self.parent[ry] = rx;
        } else {
            self.parent[ry] = rx;
            self.rank[rx] += 1;
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/duplicates",
    params(DuplicatesQuery),
    responses(
        (status = 200, description = "Duplicate groups", body = DuplicatesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    )
)]
#[get("/duplicates")]
pub async fn get_duplicates(
    req: HttpRequest,
    query: web::Query<DuplicatesQuery>,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "get_duplicates", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let threshold = query.threshold.clamp(0.80, 1.0);
    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;
    let is_admin = claims.role == "admin";
    let client = utils::get_db_client(&pool.0).await?;

    let mut image_index: HashMap<(String, String), usize> = HashMap::new();
    let mut image_meta: Vec<DuplicateImage> = Vec::new();
    let mut pairs: Vec<(usize, usize, f32)> = Vec::new();

    // Phase 1: Exact duplicates — same hash uploaded from multiple devices
    let exact_rows = if is_admin {
        client
            .query(
                "SELECT hash, deviceid, name, created_at, \
                        aesthetic_score, sharpness_score, width, height, file_size_bytes \
                 FROM images \
                 WHERE deleted_at IS NULL AND embedding IS NOT NULL \
                   AND (user_id, hash) IN ( \
                     SELECT user_id, hash FROM images \
                     WHERE deleted_at IS NULL \
                     GROUP BY user_id, hash HAVING COUNT(*) > 1 \
                   ) \
                 ORDER BY hash, added_at",
                &[],
            )
            .await
    } else {
        client
            .query(
                "SELECT hash, deviceid, name, created_at, \
                        aesthetic_score, sharpness_score, width, height, file_size_bytes \
                 FROM images \
                 WHERE user_id = $1 AND deleted_at IS NULL AND embedding IS NOT NULL \
                   AND hash IN ( \
                     SELECT hash FROM images \
                     WHERE user_id = $1 AND deleted_at IS NULL \
                     GROUP BY hash HAVING COUNT(*) > 1 \
                   ) \
                 ORDER BY hash, added_at",
                &[&user_uuid],
            )
            .await
    }
    .map_err(|e| {
        error!("Failed to query exact duplicates: {}", e);
        actix_web::error::ErrorInternalServerError("Database error")
    })?;

    let mut current_hash = String::new();
    let mut current_group: Vec<usize> = Vec::new();

    for row in &exact_rows {
        let hash: String = row.get(0);
        let deviceid: String = row.get(1);
        let name: String = row.get(2);
        let created_at: DateTime<Utc> = row.get(3);
        let aesthetic_score: Option<f32> = row.get(4);
        let sharpness_score: Option<f32> = row.get(5);
        let width: Option<i32> = row.get(6);
        let height: Option<i32> = row.get(7);
        let file_size_bytes: Option<i32> = row.get(8);

        if hash != current_hash {
            // Flush previous group
            for i in 0..current_group.len() {
                for j in (i + 1)..current_group.len() {
                    pairs.push((current_group[i], current_group[j], 1.0f32));
                }
            }
            current_hash = hash.clone();
            current_group.clear();
        }

        let key = (hash.clone(), deviceid.clone());
        let idx = if let Some(&existing) = image_index.get(&key) {
            existing
        } else {
            let new_idx = image_meta.len();
            image_meta.push(DuplicateImage {
                thumbnail_url: format!("/api/thumbnail/{}", hash),
                hash,
                deviceid,
                name,
                created_at: created_at.to_rfc3339(),
                aesthetic_score,
                sharpness_score,
                width,
                height,
                file_size_bytes,
            });
            image_index.insert(key, new_idx);
            new_idx
        };
        current_group.push(idx);
    }
    // Flush last exact group
    for i in 0..current_group.len() {
        for j in (i + 1)..current_group.len() {
            pairs.push((current_group[i], current_group[j], 1.0f32));
        }
    }

    // Phase 2: Near-duplicates via HNSW index (different hash, high cosine similarity)
    let near_rows = if is_admin {
        client
            .query(
                "SELECT DISTINCT ON (LEAST(t.hash, n.hash), GREATEST(t.hash, n.hash)) \
                 t.hash, t.deviceid, t.name, t.created_at, \
                 n.hash, n.deviceid, n.name, n.created_at, \
                 (1.0 - (t.embedding <=> n.embedding))::float4 AS similarity, \
                 t.aesthetic_score, t.sharpness_score, t.width, t.height, t.file_size_bytes, \
                 n.aesthetic_score, n.sharpness_score, n.width, n.height, n.file_size_bytes \
                 FROM images t \
                 CROSS JOIN LATERAL ( \
                     SELECT hash, deviceid, name, created_at, embedding, \
                            aesthetic_score, sharpness_score, width, height, file_size_bytes \
                     FROM images \
                     WHERE user_id = t.user_id AND deleted_at IS NULL \
                       AND embedding IS NOT NULL \
                       AND hash != t.hash \
                     ORDER BY embedding <=> t.embedding \
                     LIMIT 10 \
                 ) n \
                 WHERE t.deleted_at IS NULL AND t.embedding IS NOT NULL \
                   AND (t.embedding <=> n.embedding) < (1.0::float8 - $1::float8) \
                 ORDER BY LEAST(t.hash, n.hash), GREATEST(t.hash, n.hash), \
                          (1.0 - (t.embedding <=> n.embedding)) DESC",
                &[&threshold],
            )
            .await
    } else {
        client
            .query(
                "SELECT DISTINCT ON (LEAST(t.hash, n.hash), GREATEST(t.hash, n.hash)) \
                 t.hash, t.deviceid, t.name, t.created_at, \
                 n.hash, n.deviceid, n.name, n.created_at, \
                 (1.0 - (t.embedding <=> n.embedding))::float4 AS similarity, \
                 t.aesthetic_score, t.sharpness_score, t.width, t.height, t.file_size_bytes, \
                 n.aesthetic_score, n.sharpness_score, n.width, n.height, n.file_size_bytes \
                 FROM images t \
                 CROSS JOIN LATERAL ( \
                     SELECT hash, deviceid, name, created_at, embedding, \
                            aesthetic_score, sharpness_score, width, height, file_size_bytes \
                     FROM images \
                     WHERE user_id = t.user_id AND deleted_at IS NULL \
                       AND embedding IS NOT NULL \
                       AND hash != t.hash \
                     ORDER BY embedding <=> t.embedding \
                     LIMIT 10 \
                 ) n \
                 WHERE t.user_id = $1 AND t.deleted_at IS NULL AND t.embedding IS NOT NULL \
                   AND (t.embedding <=> n.embedding) < (1.0::float8 - $2::float8) \
                 ORDER BY LEAST(t.hash, n.hash), GREATEST(t.hash, n.hash), \
                          (1.0 - (t.embedding <=> n.embedding)) DESC",
                &[&user_uuid, &threshold],
            )
            .await
    }
    .map_err(|e| {
        error!("Failed to query near duplicates: {}", e);
        actix_web::error::ErrorInternalServerError("Database error")
    })?;

    for row in &near_rows {
        let hash1: String = row.get(0);
        let deviceid1: String = row.get(1);
        let name1: String = row.get(2);
        let created_at1: DateTime<Utc> = row.get(3);
        let hash2: String = row.get(4);
        let deviceid2: String = row.get(5);
        let name2: String = row.get(6);
        let created_at2: DateTime<Utc> = row.get(7);
        let similarity: f32 = row.get(8);
        let aesthetic_score1: Option<f32> = row.get(9);
        let sharpness_score1: Option<f32> = row.get(10);
        let width1: Option<i32> = row.get(11);
        let height1: Option<i32> = row.get(12);
        let file_size_bytes1: Option<i32> = row.get(13);
        let aesthetic_score2: Option<f32> = row.get(14);
        let sharpness_score2: Option<f32> = row.get(15);
        let width2: Option<i32> = row.get(16);
        let height2: Option<i32> = row.get(17);
        let file_size_bytes2: Option<i32> = row.get(18);

        let key1 = (hash1.clone(), deviceid1.clone());
        let idx1 = if let Some(&existing) = image_index.get(&key1) {
            existing
        } else {
            let new_idx = image_meta.len();
            image_meta.push(DuplicateImage {
                thumbnail_url: format!("/api/thumbnail/{}", hash1),
                hash: hash1,
                deviceid: deviceid1,
                name: name1,
                created_at: created_at1.to_rfc3339(),
                aesthetic_score: aesthetic_score1,
                sharpness_score: sharpness_score1,
                width: width1,
                height: height1,
                file_size_bytes: file_size_bytes1,
            });
            image_index.insert(key1, new_idx);
            new_idx
        };

        let key2 = (hash2.clone(), deviceid2.clone());
        let idx2 = if let Some(&existing) = image_index.get(&key2) {
            existing
        } else {
            let new_idx = image_meta.len();
            image_meta.push(DuplicateImage {
                thumbnail_url: format!("/api/thumbnail/{}", hash2),
                hash: hash2,
                deviceid: deviceid2,
                name: name2,
                created_at: created_at2.to_rfc3339(),
                aesthetic_score: aesthetic_score2,
                sharpness_score: sharpness_score2,
                width: width2,
                height: height2,
                file_size_bytes: file_size_bytes2,
            });
            image_index.insert(key2, new_idx);
            new_idx
        };

        pairs.push((idx1, idx2, similarity));
    }

    let n = image_meta.len();
    if n == 0 {
        return Ok(HttpResponse::Ok().json(DuplicatesResponse {
            groups: vec![],
            total_groups: 0,
        }));
    }

    let mut uf = UnionFind::new(n);
    for &(a, b, _) in &pairs {
        uf.union(a, b);
    }

    // Track max similarity per group root
    let mut group_max_sim: HashMap<usize, f32> = HashMap::new();
    for &(a, b, sim) in &pairs {
        let root = uf.find(a);
        let _ = uf.find(b); // ensure path compression
        let entry = group_max_sim.entry(root).or_insert(0.0f32);
        if sim > *entry {
            *entry = sim;
        }
    }

    // Assign each image to its group root
    let mut group_images: HashMap<usize, Vec<usize>> = HashMap::new();
    for idx in 0..n {
        let root = uf.find(idx);
        group_images.entry(root).or_default().push(idx);
    }

    let mut groups: Vec<DuplicateGroup> = group_images
        .into_iter()
        .filter(|(_, members)| members.len() >= 2)
        .map(|(root, members)| {
            let similarity = *group_max_sim.get(&root).unwrap_or(&0.0);
            let images = members.iter().map(|&i| image_meta[i].clone()).collect();
            DuplicateGroup { similarity, images }
        })
        .collect();

    groups.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap_or(std::cmp::Ordering::Equal));

    let total_groups = groups.len();
    Ok(HttpResponse::Ok().json(DuplicatesResponse { groups, total_groups }))
}
