use actix_web::{get, post, delete, web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use crate::config::Config;
use crate::utils;
use crate::db::MainDbPool;
use crate::metrics::BACKUP_PEERS_AVAILABLE;
use np2p::network::P2PService;
use std::sync::Arc;
use hex;

#[derive(Debug, Serialize, Deserialize, ToSchema, Clone)]
pub struct RebalanceProgress {
    pub is_active: bool,
    pub total_files: i64,
    pub balanced_files: i64,
    pub unbalanced_files: i64,
    pub progress_percent: f64,
    pub target_nodes: usize,
}

/// Active-peer bar above which the mesh reports full parity ("healthy").
/// Under admission control (non-empty allow-list) the mesh can never exceed the
/// admitted node count, so the bar is the allow-list size itself; an open mesh
/// keeps the absolute 3/5 Reed-Solomon total of 5.
fn mesh_full_parity_bar(admitted_nodes: usize) -> usize {
    if admitted_nodes == 0 { 5 } else { admitted_nodes }
}

/// Mesh health decision: healthy = every expected peer present; degraded = at least
/// 3 active peers (every file still reconstructable); critical = fewer than 3.
fn classify_mesh_health(active_peers: usize, full_parity_bar: usize) -> (bool, &'static str) {
    if active_peers >= full_parity_bar {
        (true, "healthy")
    } else if active_peers >= 3 {
        (true, "degraded")
    } else {
        (false, "critical")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_mesh_keeps_absolute_five_peer_bar() {
        assert_eq!(mesh_full_parity_bar(0), 5);
        assert_eq!(classify_mesh_health(5, 5), (true, "healthy"));
        assert_eq!(classify_mesh_health(4, 5), (true, "degraded"));
    }

    #[test]
    fn admitted_mesh_is_healthy_when_full() {
        // Home topology: home server + 3 Pi nodes admitted.
        let bar = mesh_full_parity_bar(4);
        assert_eq!(bar, 4);
        assert_eq!(
            classify_mesh_health(4, bar),
            (true, "healthy"),
            "fully connected 4-node mesh must not report 'degraded' forever"
        );
        assert_eq!(classify_mesh_health(3, bar), (true, "degraded"));
    }

    #[test]
    fn below_three_peers_is_critical_regardless_of_topology() {
        assert_eq!(classify_mesh_health(2, 4), (false, "critical"));
        assert_eq!(classify_mesh_health(0, 5), (false, "critical"));
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct P2PBackupStatusResponse {
    pub local_peer_id: String,
    pub is_healthy: bool,
    /// "healthy" = every expected peer active (admission allow-list size, or 5+ on an open mesh),
    /// "degraded" = 3+ active (all files still reconstructable), "critical" = <3
    pub health_status: String,
    pub active_peers: usize,
    pub total_shards_stored: i64,
    /// Synced media files with all 5 shards available on currently-reachable nodes.
    pub ok_files: i64,
    /// 3-4 shards available (reconstructable, but no parity redundancy).
    pub degraded_files: i64,
    /// 1-2 shards available (not reconstructable).
    pub failed_files: i64,
    /// 0 shards available on reachable nodes.
    pub missing_files: i64,
    /// Media files not yet replicated (p2p_synced_at IS NULL).
    pub pending_images: i64,
    pub pending_videos: i64,
    /// Rolling pg_dump snapshots distributed over the mesh (database backups).
    pub db_backups_count: i64,
    pub db_backups_total_bytes: i64,
    pub db_backups_latest_at: Option<String>,
    pub rebalance: RebalanceProgress,
}

#[utoipa::path(
    get,
    path = "/api/p2p/backup/status",
    responses(
        (status = 200, description = "P2P backup status", body = P2PBackupStatusResponse),
        (status = 401, description = "Unauthorized")
    ),
    tag = "P2P"
)]
#[get("/p2p/backup/status")]
pub async fn get_p2p_backup_status(
    req: HttpRequest,
    config: web::Data<Config>,
    p2p_service: web::Data<Arc<P2PService>>,
    pool: web::Data<MainDbPool>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "get_p2p_backup_status", config.get_api_key()).await {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    if claims.role != "admin" {
        return Ok(HttpResponse::Forbidden().json(serde_json::json!({"error": "admin required"})));
    }

    let active_nodes: Vec<(String, std::net::SocketAddr)> = p2p_service.registry.all()
        .into_iter()
        .map(|p| (p.node_id, p.addr))
        .collect();

    if !active_nodes.is_empty() {
        let _ = crate::shard_rebalance_worker::ensure_peers_registered(&pool.0, &active_nodes).await;
    }

    let client = utils::get_db_client(&pool.0).await?;

    let shard_count: i64 = client.query_one("SELECT COUNT(*) FROM p2p_shards", &[]).await
        .map(|row| row.get(0)).unwrap_or(0);

    // Recent peers (last 10 min) — an honest proxy for "currently reachable" that survives a restart.
    let recent_peer_count: i64 = client.query_one(
        "SELECT COUNT(*) FROM p2p_nodes WHERE is_active = TRUE AND last_seen > NOW() - INTERVAL '10 minutes'",
        &[],
    ).await.map(|row| row.get(0)).unwrap_or(0);

    // Prefer the live in-memory registry (nodes actually connected this process); fall back to the DB
    // so the count doesn't collapse to 0 right after a restart before discovery re-fires.
    let active_peers = if active_nodes.is_empty() { recent_peer_count as usize } else { active_nodes.len() };

    // Keep the peers-available metric fresh here too (the UI polls this endpoint every 30s), so the
    // BackupPeersUnavailable alert never fires on a stale value left by an idle replication cycle.
    BACKUP_PEERS_AVAILABLE.set(active_peers as i64);

    // Reconstruction breakdown across all synced media. Only shards whose owning node is currently
    // reachable count as available, so stale/dead node assignments don't falsely inflate the numbers.
    // Needs at least DATA_SHARDS (3) shards on distinct nodes to rebuild a file, 5 for full parity.
    let breakdown_row = client.query_one(
        "SELECT
            COUNT(*) FILTER (WHERE sc >= 5) AS ok_files,
            COUNT(*) FILTER (WHERE sc >= 3 AND sc < 5) AS degraded_files,
            COUNT(*) FILTER (WHERE sc > 0 AND sc < 3) AS failed_files,
            COUNT(*) FILTER (WHERE sc = 0) AS missing_files
         FROM (
            SELECT i.hash, COUNT(s.id) FILTER (WHERE n.node_id IS NOT NULL) AS sc
            FROM images i
            LEFT JOIN p2p_shards s ON s.file_hash = i.hash
            LEFT JOIN p2p_nodes n ON n.node_id = s.node_id
                AND n.is_active = TRUE AND n.last_seen > NOW() - INTERVAL '10 minutes'
            WHERE i.p2p_synced_at IS NOT NULL AND i.deleted_at IS NULL
            GROUP BY i.hash
            UNION ALL
            SELECT v.hash, COUNT(s.id) FILTER (WHERE n.node_id IS NOT NULL) AS sc
            FROM videos v
            LEFT JOIN p2p_shards s ON s.file_hash = v.hash
            LEFT JOIN p2p_nodes n ON n.node_id = s.node_id
                AND n.is_active = TRUE AND n.last_seen > NOW() - INTERVAL '10 minutes'
            WHERE v.p2p_synced_at IS NOT NULL AND v.deleted_at IS NULL
            GROUP BY v.hash
         ) AS combined",
        &[],
    ).await.map_err(|_| actix_web::error::ErrorInternalServerError("DB query error"))?;

    let ok_files: i64 = breakdown_row.get(0);
    let degraded_files: i64 = breakdown_row.get(1);
    let failed_files: i64 = breakdown_row.get(2);
    let missing_files: i64 = breakdown_row.get(3);

    let pending_images: i64 = client.query_one(
        "SELECT COUNT(*) FROM images WHERE p2p_synced_at IS NULL AND deleted_at IS NULL",
        &[],
    ).await.map(|row| row.get(0)).unwrap_or(0);

    let pending_videos: i64 = client.query_one(
        "SELECT COUNT(*) FROM videos WHERE p2p_synced_at IS NULL AND deleted_at IS NULL",
        &[],
    ).await.map(|row| row.get(0)).unwrap_or(0);

    let db_backup_row = client.query_one(
        "SELECT COUNT(*), COALESCE(SUM(size_bytes), 0)::BIGINT, MAX(created_at) FROM db_backups",
        &[],
    ).await.map_err(|_| actix_web::error::ErrorInternalServerError("DB query error"))?;
    let db_backups_count: i64 = db_backup_row.get(0);
    let db_backups_total_bytes: i64 = db_backup_row.get(1);
    let db_backups_latest_at: Option<chrono::DateTime<chrono::Utc>> = db_backup_row.get(2);
    let db_backups_latest_at = db_backups_latest_at.map(|t| t.to_rfc3339());

    // Full parity when every expected peer is present: allow-list size under admission
    // control, 5 for an open mesh. 3+ peers = reconstructable, <3 = critical.
    let (is_healthy, health_status) =
        classify_mesh_health(active_peers, mesh_full_parity_bar(config.p2p_allowed_node_ids.len()));

    let target_nodes = active_peers.clamp(1, crate::p2p_upload::SHARD_COUNT);
    let rebalance_row = client.query_one(
        "SELECT 
            count(*) as total_files,
            count(*) FILTER (WHERE node_count >= $1) as balanced_files,
            count(*) FILTER (WHERE node_count < $1) as unbalanced_files
         FROM (
            SELECT file_hash, count(distinct node_id) as node_count 
            FROM p2p_shards 
            GROUP BY file_hash
         ) sub;",
        &[&(target_nodes as i64)],
    ).await.map_err(|_| actix_web::error::ErrorInternalServerError("DB query error"))?;

    let reb_total: i64 = rebalance_row.get(0);
    let reb_balanced: i64 = rebalance_row.get(1);
    let reb_unbalanced: i64 = rebalance_row.get(2);
    let progress_percent = if reb_total > 0 {
        ((reb_balanced as f64 / reb_total as f64) * 100.0 * 10.0).round() / 10.0
    } else {
        100.0
    };

    let is_rebalancing = crate::shard_rebalance_worker::REBALANCE_ACTIVE.load(std::sync::atomic::Ordering::Relaxed) || (reb_unbalanced > 0 && active_peers > 1);

    let rebalance = RebalanceProgress {
        is_active: is_rebalancing,
        total_files: reb_total,
        balanced_files: reb_balanced,
        unbalanced_files: reb_unbalanced,
        progress_percent,
        target_nodes,
    };

    let response = P2PBackupStatusResponse {
        local_peer_id: hex::encode(p2p_service.identity().node_id()),
        is_healthy,
        health_status: health_status.to_string(),
        active_peers,
        total_shards_stored: shard_count,
        ok_files,
        degraded_files,
        failed_files,
        missing_files,
        pending_images,
        pending_videos,
        db_backups_count,
        db_backups_total_bytes,
        db_backups_latest_at,
        rebalance,
    };

    Ok(HttpResponse::Ok().json(response))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ConnectionInfoResponse {
    pub node_id: String,
    pub local_ip: Option<String>,
    /// Public URL to reach this server via the VPS coordinator tunnel (for Android on other networks).
    pub tunnel_url: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/p2p/connection",
    responses(
        (status = 200, description = "Get P2P connection info for QR code", body = ConnectionInfoResponse),
        (status = 401, description = "Unauthorized")
    ),
    tag = "P2P"
)]
#[get("/p2p/connection")]
pub async fn get_p2p_connection_info(
    req: HttpRequest,
    config: web::Data<Config>,
    p2p_service: web::Data<Arc<P2PService>>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "get_p2p_connection_info", config.get_api_key()).await {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    if claims.role != "admin" {
        return Ok(HttpResponse::Forbidden().json(serde_json::json!({"error": "admin required"})));
    }

    // Local IP: extracted from the Host header — what the browser actually connected to.
    // This avoids returning Docker bridge IPs (172.17.x.x) that are invisible to clients.
    let local_ip = req.connection_info().host()
        .split(':').next()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    // Tunnel URL: configured public URL to reach this server via VPS coordinator tunnel.
    let tunnel_url = config.p2p_tunnel_public_url.clone();

    Ok(HttpResponse::Ok().json(ConnectionInfoResponse {
        node_id: hex::encode(p2p_service.identity().node_id()),
        local_ip,
        tunnel_url,
    }))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DiscoveredPeer {
    pub peer_id: String,
    pub last_seen: String,
    pub is_active: bool,
    pub shard_count: i64,
    /// Host:port this node is reachable at (LAN IP or VPS address).
    pub public_addr: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DiscoveredPeersResponse {
    pub peer_count: usize,
    pub peers: Vec<DiscoveredPeer>,
}

#[utoipa::path(
    get,
    path = "/api/p2p-discovered-peers",
    responses(
        (status = 200, description = "List of discovered peers from database", body = DiscoveredPeersResponse),
        (status = 401, description = "Unauthorized")
    ),
    tag = "P2P"
)]
#[get("/p2p-discovered-peers")]
pub async fn get_discovered_peers(
    req: HttpRequest,
    config: web::Data<Config>,
    p2p_service: web::Data<Arc<P2PService>>,
    pool: web::Data<MainDbPool>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "get_discovered_peers", config.get_api_key()).await {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    if claims.role != "admin" {
        return Ok(HttpResponse::Forbidden().json(serde_json::json!({"error": "admin required"})));
    }

    let client = utils::get_db_client(&pool.0).await?;
    let rows = client.query(
        "SELECT n.node_id, n.last_seen, n.is_active, n.public_addr, COUNT(s.id) as shard_count
         FROM p2p_nodes n
         LEFT JOIN p2p_shards s ON s.node_id = n.node_id
         GROUP BY n.node_id, n.last_seen, n.is_active, n.public_addr
         ORDER BY n.last_seen DESC
         LIMIT 50",
        &[]
    ).await
        .map_err(|_| actix_web::error::ErrorInternalServerError("DB query error"))?;

    let mut peers: Vec<DiscoveredPeer> = rows.iter().map(|row| {
        let last_seen: chrono::DateTime<chrono::Utc> = row.get(1);
        DiscoveredPeer {
            peer_id: row.get(0),
            last_seen: last_seen.to_rfc3339(),
            is_active: row.get(2),
            public_addr: row.get(3),
            shard_count: row.get(4),
        }
    }).collect();

    // Merge in-memory registry peers not yet persisted to DB (e.g. before first replication)
    let db_ids: std::collections::HashSet<String> = peers.iter().map(|p| p.peer_id.clone()).collect();
    let now = chrono::Utc::now().to_rfc3339();
    for registry_peer in p2p_service.registry.all() {
        if !db_ids.contains(&registry_peer.node_id) {
            peers.push(DiscoveredPeer {
                peer_id: registry_peer.node_id,
                last_seen: now.clone(),
                is_active: true,
                public_addr: Some(registry_peer.addr.to_string()),
                shard_count: 0,
            });
        }
    }

    Ok(HttpResponse::Ok().json(DiscoveredPeersResponse {
        peer_count: peers.len(),
        peers,
    }))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct FileVerifyResult {
    pub root_hash: String,
    /// "ok" = all 5 shards, "degraded" = 3-4 shards (recoverable), "failed" = 1-2 shards, "missing" = 0 shards
    pub status: String,
    pub shards_available: i64,
    pub shards_required: i64,
    pub shards_total: i64,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct VerificationResult {
    pub total_files: i64,
    /// Files with all 5 shards available (full complement).
    pub verified_files: i64,
    /// Files with 3-4 shards available (recoverable, no parity redundancy).
    pub degraded_files: i64,
    pub failed_files: i64,
    pub missing_files: i64,
    pub files: Vec<FileVerifyResult>,
}

#[utoipa::path(
    get,
    path = "/api/p2p/backup/verify",
    responses(
        (status = 200, description = "Shard verification results", body = VerificationResult),
        (status = 401, description = "Unauthorized")
    ),
    tag = "P2P"
)]
#[get("/p2p/backup/verify")]
pub async fn verify_p2p_backup(
    req: HttpRequest,
    config: web::Data<Config>,
    pool: web::Data<MainDbPool>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "verify_p2p_backup", config.get_api_key()).await {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    if claims.role != "admin" {
        return Ok(HttpResponse::Forbidden().json(serde_json::json!({"error": "admin required"})));
    }

    let client = utils::get_db_client(&pool.0).await?;

    // Query all synced files from both images and videos with their shard counts.
    // Only count shards whose owning node is currently reachable (last_seen within 10 min and is_active),
    // so dead/stale node assignments don't falsely inflate the count.
    let rows = client.query(
        "SELECT hash, COALESCE(shard_count, 0) as shard_count FROM (
            SELECT i.hash, COUNT(s.id) FILTER (WHERE n.node_id IS NOT NULL) as shard_count
            FROM images i
            LEFT JOIN p2p_shards s ON s.file_hash = i.hash
            LEFT JOIN p2p_nodes n ON n.node_id = s.node_id
                AND n.is_active = TRUE
                AND n.last_seen > NOW() - INTERVAL '10 minutes'
            WHERE i.p2p_synced_at IS NOT NULL AND i.deleted_at IS NULL
            GROUP BY i.hash
            UNION ALL
            SELECT v.hash, COUNT(s.id) FILTER (WHERE n.node_id IS NOT NULL) as shard_count
            FROM videos v
            LEFT JOIN p2p_shards s ON s.file_hash = v.hash
            LEFT JOIN p2p_nodes n ON n.node_id = s.node_id
                AND n.is_active = TRUE
                AND n.last_seen > NOW() - INTERVAL '10 minutes'
            WHERE v.p2p_synced_at IS NOT NULL AND v.deleted_at IS NULL
            GROUP BY v.hash
        ) AS combined
        ORDER BY shard_count ASC",
        &[]
    ).await.map_err(|_| actix_web::error::ErrorInternalServerError("DB query error"))?;

    const SHARDS_REQUIRED: i64 = 3; // DATA_SHARDS — minimum needed to reconstruct
    const SHARDS_TOTAL: i64 = 5;    // TOTAL_SHARDS — full complement

    let mut files = Vec::with_capacity(rows.len());
    let mut verified_files: i64 = 0;
    let mut degraded_files: i64 = 0;
    let mut failed_files: i64 = 0;
    let mut missing_files: i64 = 0;

    for row in &rows {
        let hash: String = row.get(0);
        let shard_count: i64 = row.get(1);

        let status = if shard_count >= SHARDS_TOTAL {
            "ok"
        } else if shard_count >= SHARDS_REQUIRED {
            "degraded"
        } else if shard_count > 0 {
            "failed"
        } else {
            "missing"
        };

        match status {
            "ok" => verified_files += 1,
            "degraded" => degraded_files += 1,
            "failed" => failed_files += 1,
            _ => missing_files += 1,
        }

        files.push(FileVerifyResult {
            root_hash: hash,
            status: status.to_string(),
            shards_available: shard_count,
            shards_required: SHARDS_REQUIRED,
            shards_total: SHARDS_TOTAL,
            error: None,
        });
    }

    Ok(HttpResponse::Ok().json(VerificationResult {
        total_files: rows.len() as i64,
        verified_files,
        degraded_files,
        failed_files,
        missing_files,
        files,
    }))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct BackupListResponse {
    pub backups: Vec<BackupEntry>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct BackupEntry {
    pub filename: String,
    pub size: u64,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct BackupTimestampsResponse {
    pub timestamps: Vec<u64>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct InviteStatusResponse {
    pub is_member: bool,
    pub membership: Option<MembershipInfo>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MembershipInfo {
    pub node_id: String,
}

#[utoipa::path(
    get,
    path = "/api/p2p/backup/list",
    responses(
        (status = 200, description = "List of database backup snapshots", body = BackupListResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin required")
    ),
    tag = "P2P"
)]
#[get("/p2p/backup/list")]
pub async fn list_p2p_backups(
    req: HttpRequest,
    config: web::Data<Config>,
    pool: web::Data<MainDbPool>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "list_p2p_backups", config.get_api_key()).await {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    if claims.role != "admin" {
        return Ok(HttpResponse::Forbidden().json(serde_json::json!({"error": "admin required"})));
    }

    let client = utils::get_db_client(&pool.0).await?;
    let rows = client.query(
        "SELECT backup_hash, created_at, size_bytes FROM db_backups ORDER BY created_at DESC",
        &[],
    ).await.map_err(|_| actix_web::error::ErrorInternalServerError("DB query error"))?;

    let backups: Vec<BackupEntry> = rows.iter().map(|row| {
        let backup_hash: String = row.get(0);
        let created_at: chrono::DateTime<chrono::Utc> = row.get(1);
        BackupEntry {
            filename: format!("{}.pgdump", &backup_hash[..backup_hash.len().min(24)]),
            size: row.get::<_, i64>(2).max(0) as u64,
            created_at: created_at.to_rfc3339(),
        }
    }).collect();

    Ok(HttpResponse::Ok().json(BackupListResponse { backups }))
}

#[utoipa::path(
    get,
    path = "/api/p2p/backup/timestamps",
    responses(
        (status = 200, description = "List of database backup timestamps", body = BackupTimestampsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin required")
    ),
    tag = "P2P"
)]
#[get("/p2p/backup/timestamps")]
pub async fn list_backup_timestamps(
    req: HttpRequest,
    config: web::Data<Config>,
    pool: web::Data<MainDbPool>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "list_backup_timestamps", config.get_api_key()).await {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    if claims.role != "admin" {
        return Ok(HttpResponse::Forbidden().json(serde_json::json!({"error": "admin required"})));
    }

    let client = utils::get_db_client(&pool.0).await?;
    let rows = client.query(
        "SELECT EXTRACT(EPOCH FROM created_at)::BIGINT FROM db_backups ORDER BY created_at DESC",
        &[],
    ).await.map_err(|_| actix_web::error::ErrorInternalServerError("DB query error"))?;

    let timestamps: Vec<u64> = rows.iter().map(|row| row.get::<_, i64>(0).max(0) as u64).collect();

    Ok(HttpResponse::Ok().json(BackupTimestampsResponse { timestamps }))
}

#[utoipa::path(get, path = "/api/p2p-invite-status", responses((status = 200, description = "Invite status", body = InviteStatusResponse)), tag = "P2P")]
#[get("/p2p-invite-status")]
pub async fn get_invite_status() -> HttpResponse { HttpResponse::Ok().json(InviteStatusResponse { is_member: true, membership: None }) }

#[derive(Debug, Serialize, ToSchema)]
pub struct RemoveNodeResponse {
    pub node_id: String,
    pub removed_shards: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RebalanceResponse {
    pub status: String,
}

#[utoipa::path(
    delete,
    path = "/api/p2p/nodes/{node_id}",
    params(("node_id" = String, Path, description = "Node ID to remove")),
    responses(
        (status = 200, description = "Node removed", body = RemoveNodeResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin required"),
        (status = 404, description = "Node not found"),
        (status = 409, description = "Node is still active")
    ),
    tag = "P2P"
)]
#[delete("/p2p/nodes/{node_id}")]
pub async fn remove_p2p_node(
    req: HttpRequest,
    path: web::Path<String>,
    config: web::Data<Config>,
    pool: web::Data<MainDbPool>,
    p2p_service: web::Data<Arc<P2PService>>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "remove_p2p_node", config.get_api_key()).await {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    if claims.role != "admin" {
        return Ok(HttpResponse::Forbidden().json(serde_json::json!({"error": "admin required"})));
    }

    let node_id = path.into_inner();
    let client = utils::get_db_client(&pool.0).await?;

    let node_row = client.query_opt(
        "SELECT node_id, is_active, last_seen FROM p2p_nodes WHERE node_id = $1",
        &[&node_id],
    ).await.map_err(|_| actix_web::error::ErrorInternalServerError("DB error"))?;

    let Some(node) = node_row else {
        return Ok(HttpResponse::NotFound().json(serde_json::json!({"error": "node not found"})));
    };

    let is_active: bool = node.get(1);
    let last_seen: chrono::DateTime<chrono::Utc> = node.get(2);
    let one_hour_ago = chrono::Utc::now() - chrono::Duration::hours(1);
    if is_active && last_seen > one_hour_ago {
        return Ok(HttpResponse::Conflict().json(serde_json::json!({"error": "node is still active"})));
    }

    let removed_shards = client.execute(
        "DELETE FROM p2p_shards WHERE node_id = $1",
        &[&node_id],
    ).await.map_err(|_| actix_web::error::ErrorInternalServerError("DB error"))?;

    client.execute(
        "DELETE FROM p2p_nodes WHERE node_id = $1",
        &[&node_id],
    ).await.map_err(|_| actix_web::error::ErrorInternalServerError("DB error"))?;

    let pool_clone = pool.0.clone();
    let config_clone = config.get_ref().clone();
    let p2p_clone = p2p_service.get_ref().clone();
    tokio::spawn(async move {
        let _ = crate::shard_rebalance_worker::run_full_rebalance(&pool_clone, &config_clone, &p2p_clone).await;
    });

    Ok(HttpResponse::Ok().json(RemoveNodeResponse { node_id, removed_shards }))
}

#[utoipa::path(
    post,
    path = "/api/p2p/backup/rebalance",
    responses(
        (status = 202, description = "Rebalance triggered", body = RebalanceResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin required")
    ),
    tag = "P2P"
)]
#[post("/p2p/backup/rebalance")]
pub async fn trigger_rebalance(
    req: HttpRequest,
    config: web::Data<Config>,
    pool: web::Data<MainDbPool>,
    p2p_service: web::Data<Arc<P2PService>>,
) -> Result<HttpResponse, actix_web::Error> {
    let claims = match utils::authenticate_request(&req, "trigger_rebalance", config.get_api_key()).await {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    if claims.role != "admin" {
        return Ok(HttpResponse::Forbidden().json(serde_json::json!({"error": "admin required"})));
    }

    let pool_clone = pool.0.clone();
    let config_clone = config.get_ref().clone();
    let p2p_clone = p2p_service.get_ref().clone();
    if crate::shard_rebalance_worker::REBALANCE_ACTIVE.load(std::sync::atomic::Ordering::Relaxed) {
        return Ok(HttpResponse::Conflict().json(RebalanceResponse { status: "rebalance already in progress".to_string() }));
    }

    tokio::spawn(async move {
        let _ = crate::shard_rebalance_worker::run_full_rebalance(&pool_clone, &config_clone, &p2p_clone).await;
    });

    Ok(HttpResponse::Accepted().json(RebalanceResponse { status: "rebalance triggered".to_string() }))
}
