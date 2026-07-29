//! Periodic database backup to P2P storage nodes.
//!
//! On a fixed interval (default 24h) the worker runs `pg_dump -Fc`, treats the
//! dump as a high-priority system object — encrypts it (ChaCha20-Poly1305),
//! erasure-codes it into 3/5 Reed-Solomon shards, and pushes the shards to the
//! active P2P storage nodes via QUIC. A rolling manifest (`db_backups`) keeps
//! the last N snapshot hashes; older snapshots are pruned — their shards are
//! deleted from the remote nodes and their manifest rows removed.
//!
//! Key management: a fresh random ChaCha20 key encrypts each dump. The key is
//! stored encrypted-with-master-key both in `db_backups` and in the on-disk
//! `p2p_keys.escrow` file (outside the database) so it survives a DB loss.

use crate::config::Config;
use crate::metrics::{
    DB_BACKUP_DURATION_SECONDS, DB_BACKUP_FAILURES_TOTAL, DB_BACKUP_PRUNED_TOTAL,
    DB_BACKUP_SIZE_BYTES, DB_BACKUP_SNAPSHOTS_KEPT, DB_BACKUP_SUCCESS_TOTAL,
};
use deadpool_postgres::Pool;
use log::{error, info, warn};
use np2p::network::{Message, P2PService, Protocol};
use np2p::storage::StorageEngine;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;

/// Files larger than this are backed up via segmented streaming (caps peak RAM).
const SEGMENT_THRESHOLD: usize = 256 * 1024 * 1024; // 256 MB
/// Max bytes per StoreShardChunk protocol message (under the 20 MB protocol cap).
const CHUNK_MSG_SIZE: usize = 32 * 1024 * 1024; // 32 MB
/// Total shards produced per backup (3 data + 2 parity).
const SHARD_COUNT: usize = 5;

struct UploadedShard {
    idx: usize,
    node_id: String,
    addr: String,
    shard_hash_hex: String,
}

/// Entry point. Fixed-interval loop (not adaptive backoff): run a backup, then
/// sleep for the configured interval, respecting the shutdown token.
pub async fn db_backup_loop(
    pool: Pool,
    config: Config,
    p2p_service: Arc<P2PService>,
    shutdown_token: tokio_util::sync::CancellationToken,
) {
    info!("DB Backup Worker started (interval={}s, retention={})",
        config.workers.db_backup_interval_secs, config.workers.db_backup_retention_count);

    // Give LAN/coordinator discovery time to register storage nodes before first run.
    tokio::time::sleep(Duration::from_secs(30)).await;

    let interval = Duration::from_secs(config.workers.db_backup_interval_secs.max(60));
    loop {
        if let Err(e) = backup_cycle(&pool, &config, &p2p_service).await {
            error!("DB backup cycle failed: {}", e);
            DB_BACKUP_FAILURES_TOTAL.inc();
        }

        tokio::select! {
            _ = shutdown_token.cancelled() => {
                info!("DB Backup Worker shutting down");
                break;
            }
            _ = tokio::time::sleep(interval) => {}
        }
    }
}

/// One full backup + retention cycle.
async fn backup_cycle(
    pool: &Pool,
    config: &Config,
    p2p_service: &Arc<P2PService>,
) -> Result<bool, String> {
    if !config.workers.db_backup_enabled {
        return Ok(false);
    }
    if config.database_url.is_none() {
        return Ok(false);
    }

    let nodes: Vec<(String, SocketAddr)> = p2p_service
        .registry
        .all()
        .into_iter()
        .map(|p| (p.node_id, p.addr))
        .collect();
    if nodes.is_empty() {
        return Ok(false);
    }
    if nodes.len() < 3 {
        warn!("Only {} P2P nodes discovered. 3+ nodes recommended for 3/5 EC redundancy.", nodes.len());
    }

    let start = std::time::Instant::now();

    // 1. Dump the database (custom format) to a temp file.
    let dump_path = dump_database(config).await?;
    // Ensure the temp file is removed even on failure.
    let result = backup_dump_file(pool, config, p2p_service, &nodes, &dump_path).await;
    let _ = tokio::fs::remove_file(&dump_path).await;
    let did_work = result?;

    if did_work {
        DB_BACKUP_SUCCESS_TOTAL.inc();
        DB_BACKUP_DURATION_SECONDS.observe(start.elapsed().as_secs_f64());
    }

    // 2. Retention: prune old snapshots (remote shards + manifest rows + manifest files).
    let retention = config.workers.db_backup_retention_count.max(1);
    match prune_old_snapshots(pool, config, p2p_service, retention).await {
        Ok(pruned) if pruned > 0 => {
            info!("DB backup retention: pruned {} old snapshot(s)", pruned);
            DB_BACKUP_PRUNED_TOTAL.inc_by(pruned);
        }
        Ok(_) => {}
        Err(e) => warn!("DB backup retention pruning failed: {}", e),
    }

    // 3. Update kept-snapshots gauge.
    if let Ok(client) = pool.get().await {
        if let Ok(row) = client.query_one("SELECT COUNT(*) FROM db_backups", &[]).await {
            let count: i64 = row.get(0);
            DB_BACKUP_SNAPSHOTS_KEPT.set(count);
        }
    }

    Ok(did_work)
}

/// Dump the plaintext dump to shards, unless an identical snapshot already exists.
/// Returns Ok(true) if a new snapshot was stored.
async fn backup_dump_file(
    pool: &Pool,
    config: &Config,
    p2p_service: &Arc<P2PService>,
    nodes: &[(String, SocketAddr)],
    dump_path: &Path,
) -> Result<bool, String> {
    let metadata = tokio::fs::metadata(dump_path).await.map_err(|e| e.to_string())?;
    let file_size = metadata.len() as usize;
    if file_size == 0 {
        return Err("pg_dump produced an empty file".to_string());
    }

    // 2. Content hash of the plaintext dump (streamed, memory-efficient).
    let backup_hash = blake3_file_hex(dump_path).await?;

    // 3. Dedup: skip if an identical snapshot is already stored.
    {
        let client = pool.get().await.map_err(|e| e.to_string())?;
        let exists = client
            .query_opt("SELECT 1 FROM db_backups WHERE backup_hash = $1", &[&backup_hash])
            .await
            .map_err(|e| e.to_string())?
            .is_some();
        if exists {
            info!("DB unchanged since last snapshot (hash {}…) — skipping", &backup_hash[..16]);
            return Ok(false);
        }
    }

    DB_BACKUP_SIZE_BYTES.observe(file_size as f64);

    // 4. Encrypt + shard + upload.
    let encryption_key: [u8; 32] = rand::random();
    let data_shards = config.p2p_data_shards;
    let parity_shards = config.p2p_parity_shards;

    let (uploaded, enc_size, segment_count, segment_enc_sizes) = if file_size > SEGMENT_THRESHOLD {
        let (u, sizes) = upload_segmented(p2p_service, nodes, &backup_hash, dump_path, &encryption_key).await?;
        (u, 0i64, sizes.len() as i32, Some(sizes))
    } else {
        let data = tokio::fs::read(dump_path).await.map_err(|e| e.to_string())?;
        let (shards, enc_size) = StorageEngine::process_for_backup(&data, &encryption_key, &encryption_key, data_shards, parity_shards)
            .map_err(|e| e.to_string())?;
        let u = upload_inmemory(p2p_service, nodes, &backup_hash, shards).await?;
        (u, enc_size as i64, 1i32, None)
    };

    if uploaded.len() < data_shards {
        return Err(format!(
            "Only {}/{} shards stored for DB snapshot. Minimum {} required for reconstruction.",
            uploaded.len(), SHARD_COUNT, data_shards
        ));
    }

    // 5. Persist manifest + shard placement (transaction).
    let api_secret = config.get_api_key().map_err(|e| e.to_string())?;
    let encrypted_key = crate::utils::encrypt_key(&encryption_key, api_secret)?;

    let mut client = pool.get().await.map_err(|e| e.to_string())?;
    let trans = client.transaction().await.map_err(|e| e.to_string())?;
    trans.execute(
        "INSERT INTO db_backups (backup_hash, size_bytes, encrypted_size, data_shards, parity_shards, encryption_key, segment_count, segment_enc_sizes)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (backup_hash) DO NOTHING",
        &[
            &backup_hash,
            &(file_size as i64),
            &enc_size,
            &(data_shards as i32),
            &(parity_shards as i32),
            &encrypted_key,
            &segment_count,
            &segment_enc_sizes,
        ],
    ).await.map_err(|e| e.to_string())?;

    for shard in &uploaded {
        trans.execute(
            "INSERT INTO p2p_nodes (node_id, public_addr, is_active) VALUES ($1, $2, TRUE)
             ON CONFLICT (node_id) DO UPDATE SET public_addr = $2, is_active = TRUE, last_seen = NOW()",
            &[&shard.node_id, &shard.node_id],
        ).await.map_err(|e| e.to_string())?;
        trans.execute(
            "INSERT INTO db_backup_shards (backup_hash, shard_index, node_id, shard_hash)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (backup_hash, shard_index) DO UPDATE SET node_id = $3, shard_hash = $4",
            &[&backup_hash, &(shard.idx as i32), &shard.node_id, &shard.shard_hash_hex],
        ).await.map_err(|e| e.to_string())?;
    }
    trans.commit().await.map_err(|e| e.to_string())?;

    // 6. Append the (encrypted) key to the on-disk escrow file so it survives DB loss.
    append_key_to_escrow(config, &backup_hash, &encrypted_key);

    // 7. Write a self-contained restore manifest to disk (outside the DB) so the
    // snapshot can be restored even if the database itself is lost.
    let manifest = crate::db_restore::DbBackupManifest {
        backup_hash: backup_hash.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        size_bytes: file_size as i64,
        encrypted_size: enc_size,
        data_shards: data_shards as i32,
        parity_shards: parity_shards as i32,
        segment_count,
        segment_enc_sizes: segment_enc_sizes.clone(),
        encryption_key_hex: hex::encode(&encrypted_key),
        shards: uploaded.iter().map(|s| crate::db_restore::ManifestShard {
            index: s.idx,
            node_id: s.node_id.clone(),
            addr: s.addr.clone(),
            shard_hash: s.shard_hash_hex.clone(),
        }).collect(),
    };
    let manifest_dir = crate::db_restore::manifest_dir(&config.p2p_data_dir);
    if let Err(e) = crate::db_restore::write_manifest(&manifest_dir, &manifest) {
        warn!("Failed to write DB backup manifest to disk: {}", e);
    }

    // 8. Publish the manifest (encrypted with the api_secret-derived mesh key) to all
    // reachable nodes as a pinned object, so the restore map survives a full
    // home-server disk loss. Best-effort — local manifest + escrow already exist.
    if let Ok(api_secret) = config.get_api_key() {
        publish_manifest_to_mesh(p2p_service, nodes, &manifest, api_secret).await;
    }

    info!("DB snapshot {} stored: {} bytes, {} shards across {} node(s)",
        &backup_hash[..16], file_size, uploaded.len(), nodes.len());
    Ok(true)
}

/// Run `pg_dump -Fc` for the configured database into a temp file (async).
async fn dump_database(config: &Config) -> Result<PathBuf, String> {
    let database_url = config.database_url.as_ref().ok_or("Database URL not configured")?;
    let output_path = std::env::temp_dir().join(format!("reminisce_db_backup_{}.dump", chrono::Utc::now().timestamp()));

    let output = tokio::process::Command::new("pg_dump")
        .arg("--format=custom")
        .arg("--file")
        .arg(&output_path)
        .arg(database_url)
        .output()
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "pg_dump not found on PATH".to_string()
            } else {
                format!("Failed to run pg_dump: {}", e)
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = tokio::fs::remove_file(&output_path).await;
        return Err(format!("pg_dump failed: {}", stderr));
    }
    Ok(output_path)
}

/// Stream a file through BLAKE3 and return the hex digest (memory-efficient).
async fn blake3_file_hex(path: &Path) -> Result<String, String> {
    let mut file = tokio::fs::File::open(path).await.map_err(|e| e.to_string())?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 { break; }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Upload already-in-memory shards to rendezvous-selected nodes (small dumps).
async fn upload_inmemory(
    p2p_service: &Arc<P2PService>,
    nodes: &[(String, SocketAddr)],
    backup_hash: &str,
    shards: Vec<Vec<u8>>,
) -> Result<Vec<UploadedShard>, String> {
    let target_nodes = rendezvous(backup_hash, nodes, SHARD_COUNT.min(nodes.len()));
    let mut set = tokio::task::JoinSet::new();

    for (idx, shard_data) in shards.into_iter().enumerate() {
        let (node_id, addr) = target_nodes[idx % target_nodes.len()].clone();
        let svc = p2p_service.clone();
        set.spawn(async move {
            store_shard_on_node(&svc, addr, &shard_data).await.map(|shard_hash_hex| UploadedShard {
                idx,
                node_id,
                addr: addr.to_string(),
                shard_hash_hex,
            })
        });
    }

    let mut uploaded = Vec::new();
    while let Some(res) = set.join_next().await {
        match res {
            Ok(Ok(s)) => uploaded.push(s),
            Ok(Err(e)) => warn!("DB snapshot shard upload failed: {}", e),
            Err(e) => warn!("DB snapshot shard task panicked: {}", e),
        }
    }
    Ok(uploaded)
}

/// Store a single in-memory shard on a node with retries. Returns the shard hash hex.
async fn store_shard_on_node(
    p2p_service: &Arc<P2PService>,
    addr: SocketAddr,
    shard_data: &[u8],
) -> Result<String, String> {
    let shard_hash_bytes: [u8; 32] = blake3::hash(shard_data).into();
    let shard_hash_hex = blake3::hash(shard_data).to_hex().to_string();
    let mut last_err = String::new();

    for attempt in 1..=3 {
        if attempt > 1 {
            tokio::time::sleep(Duration::from_millis(500 * (attempt - 1))).await;
        }
        let conn = match p2p_service.connect_to_addr(addr).await {
            Ok(c) => c,
            Err(e) => { last_err = format!("connect failed: {}", e); continue; }
        };
        let (mut send, mut recv) = match conn.open_bi().await {
            Ok(s) => s,
            Err(e) => { last_err = format!("open_bi failed: {}", e); conn.close(0u32.into(), b"error"); continue; }
        };

        let token = p2p_service.identity().create_shard_token(&shard_hash_bytes);
        let req = Message::StoreShardRequest { shard_hash: shard_hash_bytes, data: shard_data.to_vec(), token };
        let attempt_result: Result<bool, String> = async {
            Protocol::send(&mut send, &req).await.map_err(|e| e.to_string())?;
            let msg = Protocol::receive(&mut recv).await.map_err(|e| e.to_string())?;
            match msg {
                Message::StoreShardResponse { success, .. } => Ok(success),
                other => Err(format!("unexpected response: {:?}", other)),
            }
        }.await;

        let _ = send.finish();
        match attempt_result {
            Ok(true) => { conn.close(0u32.into(), b"done"); return Ok(shard_hash_hex); }
            Ok(false) => last_err = "node rejected shard".to_string(),
            Err(e) => last_err = e,
        }
        conn.close(0u32.into(), b"error");
    }
    Err(last_err)
}

/// Upload a large dump via segmented streaming: per-segment encrypt+shard, with one
/// persistent QUIC stream per shard. Peak RAM stays bounded regardless of dump size.
async fn upload_segmented(
    p2p_service: &Arc<P2PService>,
    nodes: &[(String, SocketAddr)],
    backup_hash: &str,
    dump_path: &Path,
    encryption_key: &[u8; 32],
) -> Result<(Vec<UploadedShard>, Vec<i64>), String> {
    let file_hash_bytes: [u8; 32] = blake3::hash(backup_hash.as_bytes()).into();
    let target_nodes = rendezvous(backup_hash, nodes, SHARD_COUNT.min(nodes.len()));

    // One channel + task per shard (bounded channel provides backpressure).
    let mut senders: Vec<mpsc::Sender<Vec<u8>>> = Vec::with_capacity(SHARD_COUNT);
    let mut handles = Vec::with_capacity(SHARD_COUNT);
    for idx in 0..SHARD_COUNT {
        let (tx, rx) = mpsc::channel::<Vec<u8>>(2);
        senders.push(tx);
        let (node_id, addr) = target_nodes[idx % target_nodes.len()].clone();
        let svc = p2p_service.clone();
        handles.push(tokio::spawn(async move {
            stream_one_shard(&svc, addr, file_hash_bytes, idx, rx)
                .await
                .map(|shard_hash_hex| UploadedShard { idx, node_id, addr: addr.to_string(), shard_hash_hex })
        }));
    }

    // Read the dump in segments; encrypt+shard each; fan sub-shards out to the streams.
    let mut file = tokio::fs::File::open(dump_path).await.map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; SEGMENT_THRESHOLD];
    let mut segment_enc_sizes: Vec<i64> = Vec::new();
    loop {
        let n = read_full(&mut file, &mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 { break; }
        let seg_idx = segment_enc_sizes.len() as u32;
        let nonce_ctx: Vec<u8> = encryption_key.iter().chain(seg_idx.to_le_bytes().iter()).cloned().collect();
        let (sub_shards, enc_size) = StorageEngine::process_for_backup(&buf[..n], encryption_key, &nonce_ctx, 3, 2)
            .map_err(|e| e.to_string())?;
        segment_enc_sizes.push(enc_size as i64);
        for (idx, sub_shard) in sub_shards.iter().enumerate() {
            for chunk in sub_shard.chunks(CHUNK_MSG_SIZE) {
                if senders[idx].send(chunk.to_vec()).await.is_err() { break; }
            }
        }
    }
    drop(senders); // signal EOF to all shard tasks

    let mut uploaded = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(Ok(s)) => uploaded.push(s),
            Ok(Err(e)) => warn!("Segmented DB snapshot shard failed: {}", e),
            Err(e) => warn!("Segmented DB snapshot shard task panicked: {}", e),
        }
    }
    Ok((uploaded, segment_enc_sizes))
}

/// Maintain one streaming upload for a single shard index. Returns the shard hash hex.
async fn stream_one_shard(
    p2p_service: &Arc<P2PService>,
    addr: SocketAddr,
    file_hash_bytes: [u8; 32],
    idx: usize,
    mut rx: mpsc::Receiver<Vec<u8>>,
) -> Result<String, String> {
    let conn = p2p_service.connect_to_addr(addr).await.map_err(|e| e.to_string())?;
    let (mut send, mut recv) = conn.open_bi().await.map_err(|e| e.to_string())?;

    let result: Result<String, String> = async {
        Protocol::send(&mut send, &Message::StoreShardStreamInit {
            file_hash: file_hash_bytes,
            shard_index: idx as u8,
            total_shard_bytes: 0,
            segment_count: 0,
        }).await.map_err(|e| e.to_string())?;
        match Protocol::receive(&mut recv).await.map_err(|e| e.to_string())? {
            Message::StoreShardStreamAck { ready: true } => {}
            other => return Err(format!("unexpected stream ack: {:?}", other)),
        }

        let mut hasher = blake3::Hasher::new();
        while let Some(chunk) = rx.recv().await {
            hasher.update(&chunk);
            Protocol::send(&mut send, &Message::StoreShardChunk { data: chunk }).await.map_err(|e| e.to_string())?;
        }
        let shard_hash: [u8; 32] = hasher.finalize().into();
        Protocol::send(&mut send, &Message::StoreShardStreamFinal { shard_hash }).await.map_err(|e| e.to_string())?;
        match Protocol::receive(&mut recv).await.map_err(|e| e.to_string())? {
            Message::StoreShardStreamResponse { success: true } => Ok(blake3::Hash::from(shard_hash).to_hex().to_string()),
            other => Err(format!("node rejected shard stream: {:?}", other)),
        }
    }.await;

    let _ = send.finish();
    conn.close(0u32.into(), if result.is_ok() { b"done" } else { b"error" });
    result
}

/// Read until `buf` is full or EOF. Returns bytes read.
async fn read_full(file: &mut tokio::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        match file.read(&mut buf[total..]).await? {
            0 => break,
            n => total += n,
        }
    }
    Ok(total)
}

/// Rendezvous (HRW) selection of `count` nodes for a given backup hash.
fn rendezvous(backup_hash: &str, nodes: &[(String, SocketAddr)], count: usize) -> Vec<(String, SocketAddr)> {
    let mut scored: Vec<(u64, usize)> = nodes.iter().enumerate().map(|(i, (node_id, _))| {
        let h = blake3::hash(format!("{}:{}", backup_hash, node_id).as_bytes());
        (u64::from_le_bytes(h.as_bytes()[0..8].try_into().unwrap()), i)
    }).collect();
    scored.sort_by_key(|&x| std::cmp::Reverse(x.0));
    scored.into_iter().take(count).map(|(_, i)| nodes[i].clone()).collect()
}

/// Publish the snapshot manifest to every reachable node as an encrypted pinned
/// object (both the `latest` pointer and a per-snapshot name), so the restore map
/// survives a full home-server disk loss. Best-effort per node.
async fn publish_manifest_to_mesh(
    p2p_service: &Arc<P2PService>,
    nodes: &[(String, SocketAddr)],
    manifest: &crate::db_restore::DbBackupManifest,
    api_secret: &str,
) {
    let json = match serde_json::to_vec(manifest) {
        Ok(j) => j,
        Err(e) => { warn!("Mesh manifest serialize failed: {}", e); return; }
    };
    let encrypted = match crate::db_restore::encrypt_for_mesh(&json, api_secret) {
        Ok(d) => d,
        Err(e) => { warn!("Mesh manifest encrypt failed: {}", e); return; }
    };

    let names = [
        crate::db_restore::MESH_LATEST_MANIFEST.to_string(),
        crate::db_restore::mesh_manifest_name(&manifest.backup_hash),
    ];

    let mut stored = 0usize;
    for (node_id, addr) in nodes {
        for name in &names {
            let name_hash: [u8; 32] = blake3::hash(name.as_bytes()).into();
            let token = p2p_service.identity().create_shard_token(&name_hash);
            let msg = Message::StorePinnedObject { name: name.clone(), data: encrypted.clone(), token };
            match p2p_service.send_message(node_id, &msg).await {
                Ok(Message::StorePinnedResponse { success: true }) => { stored += 1; }
                Ok(other) => warn!("Mesh manifest store to {} for '{}' unexpected: {:?}", node_id, name, other),
                Err(e) => warn!("Mesh manifest store to {} for '{}' failed: {}", node_id, name, e),
            }
        }
        let _ = addr;
    }
    info!("Published DB manifest to mesh ({} copies across {} node(s))", stored, nodes.len());
}

/// Append the encrypted backup key to the on-disk escrow file (outside the DB).
fn append_key_to_escrow(config: &Config, backup_hash: &str, encrypted_key: &[u8]) {    let base_dir = config.get_images_dir();
    let abs_base = std::fs::canonicalize(base_dir).unwrap_or_else(|_| PathBuf::from(base_dir));
    let escrow_path = abs_base.parent().unwrap_or(&abs_base).join("p2p_keys.escrow");

    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    if let Ok(mut file) = options.open(&escrow_path) {
        use std::io::Write;
        let line = format!("dbbackup:{},{}\n", backup_hash, hex::encode(encrypted_key));
        let _ = file.write_all(line.as_bytes());
    } else {
        warn!("Could not open p2p_keys.escrow to record DB backup key");
    }
}

/// Delete snapshots older than the newest `retention`. Removes shards from the
/// remote nodes, then the manifest rows (cascading to db_backup_shards) and the
/// on-disk manifest files.
async fn prune_old_snapshots(
    pool: &Pool,
    config: &Config,
    p2p_service: &Arc<P2PService>,
    retention: i64,
) -> Result<u64, String> {
    let client = pool.get().await.map_err(|e| e.to_string())?;
    let old = client
        .query("SELECT backup_hash FROM db_backups ORDER BY created_at DESC OFFSET $1", &[&retention])
        .await
        .map_err(|e| e.to_string())?;

    let mut pruned = 0u64;
    for row in old {
        let backup_hash: String = row.get(0);

        // Delete each shard from its remote node (best-effort).
        let shard_rows = client
            .query("SELECT shard_hash, node_id FROM db_backup_shards WHERE backup_hash = $1", &[&backup_hash])
            .await
            .map_err(|e| e.to_string())?;
        for srow in shard_rows {
            let shard_hash: String = srow.get(0);
            let node_id: String = srow.get(1);
            delete_shard_remote(pool, p2p_service, &node_id, &shard_hash).await;
        }

        // Remove the manifest row (cascades to db_backup_shards) and the on-disk manifest file.
        client
            .execute("DELETE FROM db_backups WHERE backup_hash = $1", &[&backup_hash])
            .await
            .map_err(|e| e.to_string())?;
        crate::db_restore::delete_manifest(&crate::db_restore::manifest_dir(&config.p2p_data_dir), &backup_hash);
        pruned += 1;
    }
    Ok(pruned)
}

/// Send a DeleteShardRequest to the node holding a shard (best-effort).
async fn delete_shard_remote(
    pool: &Pool,
    p2p_service: &Arc<P2PService>,
    node_id: &str,
    shard_hash_hex: &str,
) {
    let addr = match crate::shard_rebalance_worker::lookup_node_addr(pool, p2p_service, node_id).await {
        Some(a) => a,
        None => {
            warn!("Retention: cannot resolve addr for node {} — shard {} left on node", node_id, &shard_hash_hex[..16.min(shard_hash_hex.len())]);
            return;
        }
    };

    let shard_hash_bytes: [u8; 32] = match hex::decode(shard_hash_hex).ok().and_then(|b| b.try_into().ok()) {
        Some(h) => h,
        None => { warn!("Retention: invalid shard hash hex {}", shard_hash_hex); return; }
    };

    let result: Result<bool, String> = async {
        let conn = p2p_service.connect_to_addr(addr).await.map_err(|e| e.to_string())?;
        let (mut send, mut recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
        let token = p2p_service.identity().create_shard_token(&shard_hash_bytes);
        Protocol::send(&mut send, &Message::DeleteShardRequest { shard_hash: shard_hash_bytes, token }).await.map_err(|e| e.to_string())?;
        let msg = Protocol::receive(&mut recv).await.map_err(|e| e.to_string())?;
        let _ = send.finish();
        conn.close(0u32.into(), b"done");
        match msg {
            Message::DeleteShardResponse { success, .. } => Ok(success),
            other => Err(format!("unexpected delete response: {:?}", other)),
        }
    }.await;

    match result {
        Ok(true) => info!("Retention: deleted shard {} from node {}", &shard_hash_hex[..16.min(shard_hash_hex.len())], node_id),
        Ok(false) => warn!("Retention: node {} refused to delete shard {}", node_id, &shard_hash_hex[..16.min(shard_hash_hex.len())]),
        Err(e) => warn!("Retention: failed to delete shard {} from node {}: {}", &shard_hash_hex[..16.min(shard_hash_hex.len())], node_id, e),
    }
}
