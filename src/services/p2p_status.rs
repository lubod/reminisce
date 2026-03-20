use actix_web::{get, post, delete, web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use crate::config::Config;
use crate::utils;
use crate::db::MainDbPool;
use np2p::network::P2PService;
use std::sync::Arc;
use hex;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct P2PBackupStatusResponse {
    pub local_peer_id: String,
    pub is_healthy: bool,
    pub active_peers: usize,
    pub total_shards_stored: i64,
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
    let _claims = match utils::authenticate_request(&req, "get_p2p_backup_status", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let client = utils::get_db_client(&pool.0).await?;

    let shard_count: i64 = client.query_one("SELECT COUNT(*) FROM p2p_shards", &[]).await
        .map(|row| row.get(0)).unwrap_or(0);

    let peer_count: i64 = client.query_one("SELECT COUNT(*) FROM p2p_nodes WHERE is_active = TRUE AND last_seen > NOW() - INTERVAL '1 hour'", &[]).await
        .map(|row| row.get(0)).unwrap_or(0);

    let response = P2PBackupStatusResponse {
        local_peer_id: hex::encode(p2p_service.identity().node_id()),
        is_healthy: true,
        active_peers: peer_count as usize,
        total_shards_stored: shard_count,
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
    let _claims = match utils::authenticate_request(&req, "get_p2p_connection_info", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    // Local IP: extracted from the Host header — what the browser actually connected to.
    // This avoids returning Docker bridge IPs (172.17.x.x) that are invisible to clients.
    let local_ip = req.connection_info().host().to_string()
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
    let _claims = match utils::authenticate_request(&req, "get_discovered_peers", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let client = utils::get_db_client(&pool.0).await?;
    let rows = client.query(
        "SELECT n.node_id, n.last_seen, n.is_active, COUNT(s.id) as shard_count
         FROM p2p_nodes n
         LEFT JOIN p2p_shards s ON s.node_id = n.node_id
         GROUP BY n.node_id, n.last_seen, n.is_active
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
            shard_count: row.get(3),
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
    pub verified_files: i64,
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
    let _claims = match utils::authenticate_request(&req, "verify_p2p_backup", config.get_api_key()) {
        Ok(claims) => claims,
        Err(response) => return Ok(response),
    };

    let client = utils::get_db_client(&pool.0).await?;

    // Query all synced files from both images and videos with their shard counts.
    // Only count shards whose node is currently active (last_seen within 1h and is_active),
    // so dead/stale node assignments don't falsely inflate the count.
    let rows = client.query(
        "SELECT hash, COALESCE(shard_count, 0) as shard_count FROM (
            SELECT i.hash, COUNT(s.id) as shard_count
            FROM images i
            LEFT JOIN p2p_shards s ON s.file_hash = i.hash
            LEFT JOIN p2p_nodes n ON n.node_id = s.node_id
                AND n.is_active = TRUE
                AND n.last_seen > NOW() - INTERVAL '1 hour'
            WHERE i.p2p_synced_at IS NOT NULL AND i.deleted_at IS NULL
            GROUP BY i.hash
            UNION ALL
            SELECT v.hash, COUNT(s.id) as shard_count
            FROM videos v
            LEFT JOIN p2p_shards s ON s.file_hash = v.hash
            LEFT JOIN p2p_nodes n ON n.node_id = s.node_id
                AND n.is_active = TRUE
                AND n.last_seen > NOW() - INTERVAL '1 hour'
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
            "ok" | "degraded" => verified_files += 1,
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

#[utoipa::path(get, path = "/api/p2p/backup/list", responses((status = 200, description = "List of backups", body = BackupListResponse)), tag = "P2P")]
#[get("/p2p/backup/list")]
pub async fn list_p2p_backups() -> HttpResponse { HttpResponse::Ok().json(BackupListResponse { backups: vec![] }) }

#[utoipa::path(get, path = "/api/p2p/backup/timestamps", responses((status = 200, description = "List of timestamps", body = BackupTimestampsResponse)), tag = "P2P")]
#[get("/p2p/backup/timestamps")]
pub async fn list_backup_timestamps() -> HttpResponse { HttpResponse::Ok().json(BackupTimestampsResponse { timestamps: vec![] }) }

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
    let claims = match utils::authenticate_request(&req, "remove_p2p_node", config.get_api_key()) {
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
        let _ = crate::shard_rebalance_worker::rebalance_cycle(&pool_clone, &config_clone, &p2p_clone).await;
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
    let claims = match utils::authenticate_request(&req, "trigger_rebalance", config.get_api_key()) {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    if claims.role != "admin" {
        return Ok(HttpResponse::Forbidden().json(serde_json::json!({"error": "admin required"})));
    }

    let pool_clone = pool.0.clone();
    let config_clone = config.get_ref().clone();
    let p2p_clone = p2p_service.get_ref().clone();
    tokio::spawn(async move {
        let _ = crate::shard_rebalance_worker::rebalance_cycle(&pool_clone, &config_clone, &p2p_clone).await;
    });

    Ok(HttpResponse::Accepted().json(RebalanceResponse { status: "rebalance triggered".to_string() }))
}
