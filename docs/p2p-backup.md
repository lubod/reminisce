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

`ChaCha20Poly1305` with a **deterministic nonce** derived from `blake3(key || segment_index)`. The nonce is deterministic so that re-encrypting the same file+key always produces the same ciphertext. This is critical for repair: a re-sharded shard must be byte-identical to the original so it's compatible with the 4 surviving shards.

Implementation: `np2p/src/storage/encryption.rs`

## Large Files (>256 MB)

Files larger than `SEGMENT_THRESHOLD = 256 MB` are split into segments before sharding. Each segment is independently encrypted and erasure-coded. The resulting shards on each Pi are the concatenation of the per-segment sub-shards:

```
shard_i = [seg0_sub_shard_i][seg1_sub_shard_i]...[segN_sub_shard_i]
```

The segment sizes are stored in `p2p_segment_enc_sizes BIGINT[]` and the count in `p2p_segment_count INTEGER` on the `images`/`videos` rows.

## Workers

### Replication Worker (`media_replication_worker.rs`)

Runs every 30 seconds. Picks up files where `p2p_synced_at IS NULL`:
1. Rendezvous-select 5 target nodes
2. Encrypt + shard the file
3. Upload each shard via QUIC (`Message::StoreShardRequest`)
4. On success: set `p2p_synced_at = NOW()`, insert 5 rows into `p2p_shards`

### Audit Worker (`p2p_audit_worker.rs`)

Runs every 7 days. For each synced file:
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

## Coordinator (WAN Connectivity)

When Android is not on the home LAN, it connects to the home server via a reverse QUIC tunnel through the coordinator (a small VPS process). The coordinator also maintains a peer registry so storage nodes can find each other across NATs.

See `coordinator/src/main.rs` for the full coordinator implementation.
