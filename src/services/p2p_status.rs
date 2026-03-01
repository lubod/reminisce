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
    let rows = client.query("SELECT node_id, last_seen, is_active FROM p2p_nodes ORDER BY last_seen DESC LIMIT 50", &[]).await
        .map_err(|_| actix_web::error::ErrorInternalServerError("DB query error"))?;

    let peers = rows.iter().map(|row| {
        let last_seen: chrono::DateTime<chrono::Utc> = row.get(1);
        DiscoveredPeer {
            peer_id: row.get(0),
            last_seen: last_seen.to_rfc3339(),
            is_active: row.get(2),
        }
    }).collect::<Vec<_>>();

    Ok(HttpResponse::Ok().json(DiscoveredPeersResponse {
        peer_count: peers.len(),
        peers,
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

// Stubs for remaining endpoints
#[utoipa::path(get, path = "/api/p2p/backup/verify", responses((status = 200, description = "Feature pending")), tag = "P2P")]
#[get("/p2p/backup/verify")]
pub async fn verify_p2p_backup() -> HttpResponse { HttpResponse::Ok().json(serde_json::json!({"status": "feature_pending"})) }

#[utoipa::path(get, path = "/api/p2p/backup/list", responses((status = 200, description = "List of backups", body = BackupListResponse)), tag = "P2P")]
#[get("/p2p/backup/list")]
pub async fn list_p2p_backups() -> HttpResponse { HttpResponse::Ok().json(BackupListResponse { backups: vec![] }) }

#[utoipa::path(get, path = "/api/p2p/backup/timestamps", responses((status = 200, description = "List of timestamps", body = BackupTimestampsResponse)), tag = "P2P")]
#[get("/p2p/backup/timestamps")]
pub async fn list_backup_timestamps() -> HttpResponse { HttpResponse::Ok().json(BackupTimestampsResponse { timestamps: vec![] }) }

#[utoipa::path(get, path = "/api/p2p-invite-status", responses((status = 200, description = "Invite status", body = InviteStatusResponse)), tag = "P2P")]
#[get("/p2p-invite-status")]
pub async fn get_invite_status() -> HttpResponse { HttpResponse::Ok().json(InviteStatusResponse { is_member: true, membership: None }) }
