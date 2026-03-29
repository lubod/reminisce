use actix_web::{get, post, web, HttpRequest, HttpResponse};
use log::error;
use serde::{Serialize, Deserialize};
use utoipa::{ToSchema, IntoParams};
use std::collections::HashMap;
use chrono::{DateTime, Utc};

use crate::config::Config;
use crate::utils;
use crate::db::MainDbPool;
use crate::duplicate_worker::SharedDuplicateStatus;

// ── Data types ────────────────────────────────────────────────────────────────

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
    pub page: usize,
    pub limit: usize,
}

#[derive(Serialize, ToSchema)]
pub struct DuplicateWorkerStatusResponse {
    pub running: bool,
    pub checked_images: i64,
    pub total_images: i64,
    pub total_pairs: i64,
    pub last_completed_at: Option<String>,
}

// ── Query params ──────────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
pub struct DuplicatesQuery {
    /// Cosine similarity threshold (0.80–1.0). Default 0.95.
    #[serde(default = "default_threshold")]
    pub threshold: f64,
    /// Page number (1-based). Default 1.
    #[serde(default = "default_page")]
    pub page: usize,
    /// Groups per page. Default 20.
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_threshold() -> f64 { 0.95 }
fn default_page() -> usize { 1 }
fn default_limit() -> usize { 20 }

// ── Union-Find ────────────────────────────────────────────────────────────────

struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        UnionFind { parent: (0..n).collect(), rank: vec![0; n] }
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }

    fn union(&mut self, x: usize, y: usize) {
        let (rx, ry) = (self.find(x), self.find(y));
        if rx == ry { return; }
        if self.rank[rx] < self.rank[ry] { self.parent[rx] = ry; }
        else if self.rank[rx] > self.rank[ry] { self.parent[ry] = rx; }
        else { self.parent[ry] = rx; self.rank[rx] += 1; }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Build groups from a flat list of pairs using union-find, then
/// enrich with metadata from the images table.
async fn build_groups(
    pairs: Vec<(String, String, f32)>,   // (hash_a, hash_b, similarity)
    is_admin: bool,
    user_uuid: uuid::Uuid,
    client: &deadpool_postgres::Object,
) -> Result<Vec<DuplicateGroup>, Box<dyn std::error::Error>> {
    if pairs.is_empty() {
        return Ok(vec![]);
    }

    // Collect unique hashes
    let mut hash_index: HashMap<String, usize> = HashMap::new();
    let mut hashes: Vec<String> = Vec::new();
    for (ha, hb, _) in &pairs {
        for h in [ha, hb] {
            if !hash_index.contains_key(h) {
                hash_index.insert(h.clone(), hashes.len());
                hashes.push(h.clone());
            }
        }
    }

    let n = hashes.len();
    let mut uf = UnionFind::new(n);
    let mut group_max_sim: HashMap<usize, f32> = HashMap::new();

    for (ha, hb, sim) in &pairs {
        let ia = hash_index[ha];
        let ib = hash_index[hb];
        uf.union(ia, ib);
        let root = uf.find(ia);
        let e = group_max_sim.entry(root).or_insert(0.0f32);
        if *sim > *e { *e = *sim; }
    }

    // Group hashes by root
    let mut group_members: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        let root = uf.find(i);
        group_members.entry(root).or_default().push(i);
    }

    // Fetch metadata for all involved hashes in one query
    let hash_refs: Vec<&str> = hashes.iter().map(|s| s.as_str()).collect();
    let meta_rows = if is_admin {
        client.query(
            "SELECT DISTINCT ON (hash) hash, deviceid, name, created_at, \
                    aesthetic_score, sharpness_score, width, height, file_size_bytes \
             FROM images \
             WHERE hash = ANY($1) AND deleted_at IS NULL \
             ORDER BY hash, added_at",
            &[&hash_refs],
        ).await?
    } else {
        client.query(
            "SELECT DISTINCT ON (hash) hash, deviceid, name, created_at, \
                    aesthetic_score, sharpness_score, width, height, file_size_bytes \
             FROM images \
             WHERE hash = ANY($1) AND user_id = $2 AND deleted_at IS NULL \
             ORDER BY hash, added_at",
            &[&hash_refs, &user_uuid],
        ).await?
    };

    let mut meta_map: HashMap<String, DuplicateImage> = HashMap::new();
    for row in &meta_rows {
        let hash: String = row.get(0);
        let created_at: DateTime<Utc> = row.get(3);
        meta_map.insert(hash.clone(), DuplicateImage {
            thumbnail_url: format!("/api/thumbnail/{}", hash),
            hash,
            deviceid: row.get(1),
            name: row.get(2),
            created_at: created_at.to_rfc3339(),
            aesthetic_score: row.get(4),
            sharpness_score: row.get(5),
            width: row.get(6),
            height: row.get(7),
            file_size_bytes: row.get(8),
        });
    }

    let mut groups: Vec<DuplicateGroup> = group_members
        .into_iter()
        .filter(|(_, members)| members.len() >= 2)
        .filter_map(|(root, members)| {
            let similarity = *group_max_sim.get(&root).unwrap_or(&0.0);
            let images: Vec<DuplicateImage> = members.iter()
                .filter_map(|&i| meta_map.get(&hashes[i]).cloned())
                .collect();
            if images.len() < 2 { return None; }
            Some(DuplicateGroup { similarity, images })
        })
        .collect();

    groups.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap_or(std::cmp::Ordering::Equal));
    Ok(groups)
}

// ── GET /api/duplicates ───────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/api/duplicates",
    params(DuplicatesQuery),
    responses(
        (status = 200, description = "Duplicate groups (paginated)", body = DuplicatesResponse),
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
        Ok(c) => c,
        Err(r) => return Ok(r),
    };

    let threshold = query.threshold.clamp(0.80, 1.0);
    let page = query.page.max(1);
    let limit = query.limit.clamp(1, 200);
    let offset = (page - 1) * limit;

    let user_uuid = utils::parse_user_uuid(&claims.user_id)?;
    let is_admin = claims.role == "admin";
    let client = utils::get_db_client(&pool.0).await?;

    let threshold_f32 = threshold as f32;

    // ── Phase 1: Exact duplicates (same hash, multiple devices/rows, same user) ──
    let exact_hash_rows = if is_admin {
        client.query(
            "SELECT hash FROM images \
             WHERE deleted_at IS NULL \
             GROUP BY user_id, hash HAVING COUNT(*) > 1",
            &[],
        ).await
    } else {
        client.query(
            "SELECT hash FROM images \
             WHERE user_id = $1 AND deleted_at IS NULL \
             GROUP BY hash HAVING COUNT(*) > 1",
            &[&user_uuid],
        ).await
    }.map_err(|e| {
        error!("get_duplicates: exact query failed: {}", e);
        actix_web::error::ErrorInternalServerError("Database error")
    })?;

    let mut exact_groups: Vec<DuplicateGroup> = Vec::new();

    if !exact_hash_rows.is_empty() {
        let exact_hashes: Vec<String> = exact_hash_rows.iter().map(|r| r.get::<_, String>(0)).collect();
        let exact_hash_refs: Vec<&str> = exact_hashes.iter().map(|s| s.as_str()).collect();

        let detail_rows = if is_admin {
            client.query(
                "SELECT hash, deviceid, name, created_at, \
                        aesthetic_score, sharpness_score, width, height, file_size_bytes \
                 FROM images \
                 WHERE hash = ANY($1) AND deleted_at IS NULL \
                 ORDER BY hash, added_at",
                &[&exact_hash_refs],
            ).await
        } else {
            client.query(
                "SELECT hash, deviceid, name, created_at, \
                        aesthetic_score, sharpness_score, width, height, file_size_bytes \
                 FROM images \
                 WHERE hash = ANY($1) AND user_id = $2 AND deleted_at IS NULL \
                 ORDER BY hash, added_at",
                &[&exact_hash_refs, &user_uuid],
            ).await
        }.map_err(|e| {
            error!("get_duplicates: exact detail query failed: {}", e);
            actix_web::error::ErrorInternalServerError("Database error")
        })?;

        let mut by_hash: HashMap<String, Vec<DuplicateImage>> = HashMap::new();
        for row in &detail_rows {
            let hash: String = row.get(0);
            let created_at: DateTime<Utc> = row.get(3);
            by_hash.entry(hash.clone()).or_default().push(DuplicateImage {
                thumbnail_url: format!("/api/thumbnail/{}", hash),
                hash,
                deviceid: row.get(1),
                name: row.get(2),
                created_at: created_at.to_rfc3339(),
                aesthetic_score: row.get(4),
                sharpness_score: row.get(5),
                width: row.get(6),
                height: row.get(7),
                file_size_bytes: row.get(8),
            });
        }

        for (_, images) in by_hash {
            if images.len() >= 2 {
                exact_groups.push(DuplicateGroup { similarity: 1.0, images });
            }
        }
    }

    let mut pairs: Vec<(String, String, f32)> = Vec::new();

    // ── Phase 2: Near-duplicates from pre-computed pairs table ──
    let near_rows = if is_admin {
        client.query(
            "SELECT hash_a, hash_b, similarity \
             FROM image_duplicate_pairs \
             WHERE similarity >= $1",
            &[&threshold_f32],
        ).await
    } else {
        client.query(
            "SELECT hash_a, hash_b, similarity \
             FROM image_duplicate_pairs \
             WHERE user_id = $1 AND similarity >= $2",
            &[&user_uuid, &threshold_f32],
        ).await
    }.map_err(|e| {
        error!("get_duplicates: near-dup query failed: {}", e);
        actix_web::error::ErrorInternalServerError("Database error")
    })?;

    for row in &near_rows {
        pairs.push((row.get(0), row.get(1), row.get(2)));
    }

    // Build near-dup groups and merge with exact groups
    let mut all_groups = exact_groups;

    if !pairs.is_empty() {
        let near_groups = build_groups(pairs, is_admin, user_uuid, &client)
            .await
            .map_err(|e| {
                error!("get_duplicates: build_groups failed: {}", e);
                actix_web::error::ErrorInternalServerError("Database error")
            })?;
        all_groups.extend(near_groups);
    }

    // Sort: exact first (sim=1.0), then by descending similarity
    all_groups.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap_or(std::cmp::Ordering::Equal));

    if all_groups.is_empty() {
        return Ok(HttpResponse::Ok().json(DuplicatesResponse {
            groups: vec![],
            total_groups: 0,
            page,
            limit,
        }));
    }

    let total_groups = all_groups.len();
    let groups = all_groups.into_iter().skip(offset).take(limit).collect();

    Ok(HttpResponse::Ok().json(DuplicatesResponse { groups, total_groups, page, limit }))
}

// ── GET /api/duplicates/status ────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/api/duplicates/status",
    responses(
        (status = 200, description = "Worker status", body = DuplicateWorkerStatusResponse),
        (status = 401, description = "Unauthorized"),
    )
)]
#[get("/duplicates/status")]
pub async fn get_duplicate_status(
    req: HttpRequest,
    config: web::Data<Config>,
    status: web::Data<SharedDuplicateStatus>,
) -> Result<HttpResponse, actix_web::Error> {
    let _ = match utils::authenticate_request(&req, "get_duplicate_status", config.get_api_key()) {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    let s = status.lock().await;
    Ok(HttpResponse::Ok().json(DuplicateWorkerStatusResponse {
        running: s.running,
        checked_images: s.checked_images,
        total_images: s.total_images,
        total_pairs: s.total_pairs,
        last_completed_at: s.last_completed_at.map(|t| t.to_rfc3339()),
    }))
}

// ── POST /api/duplicates/scan ─────────────────────────────────────────────────

#[utoipa::path(
    post,
    path = "/api/duplicates/scan",
    responses(
        (status = 200, description = "Scan triggered"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — admin only"),
    )
)]
#[post("/duplicates/scan")]
pub async fn trigger_duplicate_scan(
    req: HttpRequest,
    pool: web::Data<MainDbPool>,
    config: web::Data<Config>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "trigger_duplicate_scan", config.get_api_key()) {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    if claims.role != "admin" {
        return Ok(HttpResponse::Forbidden().json(serde_json::json!({"error": "Admin only"})));
    }

    let client = utils::get_db_client(&pool.0).await?;
    // Reset duplicates_checked_at so worker will re-scan all images
    client.execute(
        "UPDATE images SET duplicates_checked_at = NULL WHERE deleted_at IS NULL",
        &[],
    ).await.map_err(|e| {
        error!("trigger_duplicate_scan: reset failed: {}", e);
        actix_web::error::ErrorInternalServerError("Database error")
    })?;
    // Clear stale pairs
    client.execute("TRUNCATE TABLE image_duplicate_pairs", &[]).await.map_err(|e| {
        error!("trigger_duplicate_scan: truncate failed: {}", e);
        actix_web::error::ErrorInternalServerError("Database error")
    })?;

    Ok(HttpResponse::Ok().json(serde_json::json!({"status": "scan triggered"})))
}
