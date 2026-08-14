# P2P Backup System

## Overview

Each media file is encrypted and split into 5 shards using Reed-Solomon erasure coding. The shards are stored across your storage nodes (Raspberry Pis). Any 3 of 5 shards are sufficient to reconstruct the original file — you can lose 2 nodes without data loss.

## Erasure Coding (3/5 Reed-Solomon)

```
Original file (e.g. 10 MB JPEG)
        │
        ▼
  Encrypt with ChaCha20Poly1305
  (key = random 32 bytes stored in DB)
        │
        ▼
  Reed-Solomon encode → 5 shards
  [shard 0] [shard 1] [shard 2]  ← data shards (3)
  [shard 3] [shard 4]             ← parity shards (2)
        │
        ▼
  Upload each shard to a different Pi node
```

Constants (in `np2p/src/storage/mod.rs`):
- `DATA_SHARDS = 3`
- `PARITY_SHARDS = 2`
- `TOTAL_SHARDS = 5`

## Node Selection (Rendezvous Hashing)

Each file is deterministically assigned to nodes using rendezvous / highest-random-weight (HRW) hashing:

```rust
score = blake3(file_hash || node_id)
```

The 5 nodes with the highest scores receive the shards. This is stable — adding a new node only displaces a small fraction of files (1/N on average), minimizing rebalance work.

## Encryption

`ChaCha20Poly1305` with a **deterministic nonce** derived from
`blake3(key ‖ segment_index ‖ content_hash)` (the plaintext is included so two distinct
payloads can never reuse a nonce under the same key+context; identical content + key +
context reproduces byte-identical ciphertext for shard-repair compatibility).

Implementation: `np2p/src/storage/encryption.rs`

## Shard token authorization

Every store/retrieve/delete requires a signed token from the home server's P2P
identity. Tokens are **operation-bound** — the signature covers
`op_tag ‖ shard_hash ‖ timestamp` (`ShardOp::{Store, Retrieve, Delete}`) — so a
captured retrieve token can never be replayed to delete the shard, and the ±5-minute
timestamp window bounds replay. Storage nodes are **owner-pinned** via
`--authorized-node-id` (or run explicitly unpinned with `--allow-unpinned`; see
[deployment.md](deployment.md)).

## Large Files (>256 MB)

Files larger than `SEGMENT_THRESHOLD = 256 MB` are split into segments before sharding. Each segment is independently encrypted and erasure-coded. The resulting shards on each Pi are the concatenation of the per-segment sub-shards:

```
shard_i = [seg0_sub_shard_i][seg1_sub_shard_i]...[segN_sub_shard_i]
```

The segment sizes are stored in `p2p_segment_enc_sizes BIGINT[]` and the count in `p2p_segment_count INTEGER` on the `images`/`videos` rows.

## Workers

### Replication Worker (`media_replication_worker.rs`)

Runs on an adaptive backoff loop (config `workers.replication_min_secs`/`max_secs`, default 10–60 s). Picks up files where `p2p_synced_at IS NULL`:
1. Rendezvous-select 5 target nodes
2. Encrypt + shard the file
3. Upload each shard via QUIC (`Message::StoreShardRequest`)
4. On success: set `p2p_synced_at = NOW()`, insert 5 rows into `p2p_shards`

**Attempt backoff (`p2p_last_attempt_at`):** every attempt (success, missing-on-disk, or failure) stamps `p2p_last_attempt_at = NOW()`. Batch selection and `requeue_under_replicated` only pick files whose last attempt is older than 10 minutes. This prevents a node outage from re-encrypting and re-uploading the same files every cycle, and stops a permanently-missing file from occupying every batch slot (head-of-line blocking).

### Audit Worker (`p2p_audit_worker.rs`)

Runs on an adaptive backoff loop (config `workers.audit_min_secs`/`max_secs`, default 60–3600 s). Audits shards that haven't been verified in 7 days (batches of 50) and repairs each lost shard:
1. **Orphan cleanup** — delete `p2p_shards` rows for soft-deleted files
2. **Consistency check** — count shards per file; flag files with < 3 shards
3. **Repair** — for each under-sharded file, re-encrypt and re-upload the missing shard(s) to the correct node(s)

Large file repair streams the file in 256 MB segments, re-encrypts each with the stored key, extracts the sub-shard for the failed index, concatenates, and uploads.

### Rebalance Worker (`shard_rebalance_worker.rs`)

Triggered manually via `POST /api/p2p/backup/rebalance`. Migrates shards from their current node to the ideal rendezvous node when the node set has changed (e.g. a new Pi was added).

## Database Tables

`p2p_nodes` — known storage nodes:
```
node_id VARCHAR(64)   — Ed25519 public key in hex
public_addr VARCHAR   — last known address
is_active BOOLEAN
```

`p2p_shards` — shard placement map:
```
file_hash VARCHAR     — BLAKE3 hash of the media file
shard_index INTEGER   — 0–4
node_id VARCHAR       — which node holds this shard
shard_hash VARCHAR    — BLAKE3 hash of the shard itself (for integrity verification)
last_checked_at       — when this shard was last verified
```

## Restore

HTTP: `POST /api/p2p/restore/{hash}` — streams the file back as an attachment.

CLI: `cargo run --bin p2p_restore -- --hash <hex> --output /path/`

The restore logic (`src/p2p_restore.rs`) fetches all 5 shards concurrently, tolerates up to 2 missing, then reconstructs via `StorageEngine::restore_from_backup`.

## Database Snapshots (`db_backup_worker.rs`)

In addition to media files, the server periodically backs up the PostgreSQL database itself through the same P2P pipeline.

**Periodic trigger** — every 24 h (configurable via `workers.db_backup_interval_secs`) the worker runs `pg_dump -Fc` to a temp file. If the dump's BLAKE3 hash matches an existing snapshot, it's skipped (no-op when the DB hasn't changed).

**Pipeline** — the dump is treated as a high-priority system object: a fresh random ChaCha20-Poly1305 key encrypts it, it's split into 3/5 Reed-Solomon shards (segmented streaming for dumps > 256 MB), and the shards are uploaded to rendezvous-selected nodes. The per-snapshot key is stored encrypted-with-master-key in `db_backups` and appended to the on-disk `p2p_keys.escrow` file (outside the DB, so it survives a database loss).

**Retention** — the `db_backups` table is a rolling manifest. After each snapshot, snapshots older than the newest `workers.db_backup_retention_count` (default 7) are pruned: each of their shards is deleted from the remote node via a `DeleteShardRequest` protocol message, and the manifest rows are removed (cascading to `db_backup_shards`).

Shard placement is tracked in `db_backup_shards` (separate from media `p2p_shards`) so the audit worker's orphan cleanup never touches DB-snapshot shards.

**On-disk manifest** — because `db_backups`/`db_backup_shards` live inside the database being backed up, a full DB loss would also wipe the restore map. To survive that, every snapshot also writes a self-contained JSON manifest to `<p2p_data_dir>/db_manifests/<backup_hash>.json` holding shard placement, node addresses, segment layout, and the (master-key-encrypted) key. Retention pruning deletes the manifest file too.

### True full-disk-loss protection

The on-disk manifest and `node.key` still live on the home server, so losing the *entire disk* would remove them. Two mechanisms close that gap so recovery needs only the **master secret (`api_secret_key`) + the P2P nodes**:

1. **Deterministic P2P identity** — set `p2p_deterministic_identity: true`. The node identity is then derived from `api_secret_key` (`NodeIdentity::from_secret`) instead of a random `node.key`, so the node_id — and the ability to authenticate to storage nodes — is recoverable from the master secret alone. **Note:** enabling changes the node_id, so storage nodes must (re)pin it as their authorized owner; use for new setups or after re-registering nodes.
2. **Mesh-published manifest** — after each snapshot, the worker encrypts the restore manifest with an api_secret-derived key and stores it on *every* reachable node as a pinned, name-addressed object (`reminisce:db-manifest:latest` and `reminisce:db-manifest:<backup_hash>`). The restore map thus lives on the mesh, not just on disk.

With both enabled, a brand-new machine with only `config.yaml` (api_secret + coordinator/LAN) can rebuild everything: derive the identity, fetch the manifest from any node, restore the DB (recovering media keys), then restore media.

## Disaster Recovery (`src/bin/disaster_recovery.rs`)

A CLI for restoring from the P2P mesh across data-loss scenarios. It authenticates to storage nodes with the home server's P2P identity (`<p2p_data_dir>/node.key`, or derived from `api_secret_key` when `p2p_deterministic_identity` is set and `node.key` is absent) — required for the nodes to accept `RetrieveShard` tokens.

```bash
# List available DB snapshots (from on-disk manifests)
disaster_recovery list --config config.yaml

# Full DB loss: reconstruct + decrypt the latest snapshot, then pg_restore
disaster_recovery db --config config.yaml --pg-restore --target-db-url postgres://user:pass@host/reminisce_db --clean

# TRUE full-disk loss (no node.key, no manifests): pull the manifest from the mesh first
disaster_recovery db --config config.yaml --from-mesh --pg-restore --target-db-url <url> --clean

# Only media lost (DB intact): restore every file missing on disk
disaster_recovery media --config config.yaml --all-missing

# Restore a single file, or the whole library
disaster_recovery media --config config.yaml --hash <hex>
disaster_recovery media --config config.yaml --all

# Full recovery: DB snapshot first, then missing media
disaster_recovery full --config config.yaml --pg-restore --target-db-url <url>
```

`--from-mesh` discovers storage nodes via LAN broadcast (and `--node node_id=host:port`, repeatable) and retrieves the manifest from the mesh when no local copy exists. For WAN recovery (nodes not on the LAN), pass `--node` to reach at least one node directly.

## Coordinator (WAN Connectivity)

When Android is not on the home LAN, it connects to the home server via a reverse QUIC tunnel through the coordinator (a small VPS process). The coordinator also maintains a peer registry so storage nodes can find each other across NATs.

See `coordinator/src/main.rs` for the full coordinator implementation.

## Peer Identity & LAN Address Synchronization

When nodes communicate across local LANs or behind NAT tunnels, the in-memory `P2PService.registry` is automatically kept in sync with the PostgreSQL `p2p_nodes` table via `sync_db_nodes_to_registry()`.

- **Registry Precedence**: In-memory discovered active connections take immediate precedence.
- **Database Fallback**: When establishing a direct connection or repair transfer to a node ID not yet registered in memory, `lookup_node_addr()` falls back to PostgreSQL and immediately registers the peer in `P2PService.registry`, guaranteeing peer identity validation succeeds without rejection.

## Orphaned Shard Audit & Repair

The `P2P Audit Worker` periodically sweeps for inconsistent or degraded shard states:

1. **Undersharded Files**: Scans for files with fewer than 5 shards across reachable nodes and triggers repair via `p2p_audit_worker::repair_file`.
2. **Database Orphan Cleanup**: Removes rows in `p2p_shards` belonging to media files that have been deleted from the database.
3. **Storage Node Sweeps**: Queries remote storage node catalogs to identify and prune orphaned shard files residing on disk that have no active owner in PostgreSQL.
