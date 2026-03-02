use actix_web::{get, web, HttpRequest, HttpResponse};
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

    Ok(HttpResponse::Ok().json(ConnectionInfoResponse {
        node_id: hex::encode(p2p_service.identity().node_id()),
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

    let peers = rows.iter().map(|row| {
        let last_seen: chrono::DateTime<chrono::Utc> = row.get(1);
        DiscoveredPeer {
            peer_id: row.get(0),
            last_seen: last_seen.to_rfc3339(),
            is_active: row.get(2),
            shard_count: row.get(3),
        }
    }).collect::<Vec<_>>();

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

    // Query all synced files from both images and videos with their shard counts
    let rows = client.query(
        "SELECT hash, COALESCE(shard_count, 0) as shard_count FROM (
            SELECT i.hash, COUNT(s.id) as shard_count
            FROM images i
            LEFT JOIN p2p_shards s ON s.file_hash = i.hash
            WHERE i.p2p_synced_at IS NOT NULL AND i.deleted_at IS NULL
            GROUP BY i.hash
            UNION ALL
            SELECT v.hash, COUNT(s.id) as shard_count
            FROM videos v
            LEFT JOIN p2p_shards s ON s.file_hash = v.hash
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
