# Database

## Engine

PostgreSQL with extensions:
- **pgvector** — 1152-dim SigLIP2 image embeddings + 512-dim InsightFace face embeddings; HNSW index for approximate nearest-neighbour search
- **PostGIS** — `GEOGRAPHY(POINT, 4326)` for GPS coordinates; spatial index for geo radius queries

Connection pooling via `deadpool-postgres`. Two pools: one for the main DB, one for the offline geocoding DB (separate PostGIS instance).

## Tables

| Table | Purpose |
|-------|---------|
| `users` | Authentication; `role` is `admin` or `user`; `is_active` for soft-disable |
| `images` | One row per unique hash per user; primary key is `(user_id, hash)` |
| `videos` | Same structure as `images`; no embedding column (videos aren't embedded) |
| `media_sources` | History of which device uploaded a given hash; enables cross-device dedup without duplicating media rows |
| `starred_images` / `starred_videos` | Per-user stars; separate table to avoid write contention on media rows |
| `faces` | Bounding boxes + 512-dim embeddings from InsightFace; FK to `images` |
| `persons` | Face clusters; optional `name`; `representative_embedding` is the cluster centroid |
| `labels` | User-defined tags; `image_labels` / `video_labels` are the M2M join tables |
| `image_duplicate_pairs` | Pre-computed near-duplicate pairs (hash_a < hash_b invariant); populated by `duplicate_worker` |
| `ai_settings` | Per-user toggles for AI features (descriptions, embeddings, face detection, backup) |
| `p2p_nodes` | Known storage nodes — Ed25519 node ID + last known address |
| `p2p_shards` | Shard placement map — which node holds which shard for which file |

## Key Columns on `images` / `videos`

| Column | Purpose |
|--------|---------|
| `hash` | BLAKE3 content hash — the natural key for deduplication and P2P addressing |
| `deleted_at` | Soft delete timestamp; `NULL` = active. All queries filter `WHERE deleted_at IS NULL` |
| `p2p_synced_at` | `NULL` = not yet replicated to P2P; set to `NOW()` after successful shard upload |
| `p2p_encryption_key` | 32-byte ChaCha20Poly1305 key stored in DB; needed for repair and restore |
| `p2p_encrypted_size` | Ciphertext size before erasure coding; needed to reconstruct segment boundaries |
| `p2p_segment_count` | `1` for files ≤ 256 MB; `> 1` for large files split across segments |
| `p2p_segment_enc_sizes` | Array of per-segment encrypted sizes (parallel to segment index) |
| `embedding` | 1152-dim vector; `NULL` until AI worker processes the image |
| `verification_status` | `0` = pending, `1` = OK, `-1` = failed (hash mismatch on disk) |

## Index Strategy

Partial indexes on `deleted_at IS NULL` are used throughout to keep active-record queries fast without scanning soft-deleted rows:

```sql
CREATE INDEX idx_images_deleted_at ON images(deleted_at) WHERE deleted_at IS NULL;
CREATE INDEX idx_images_user_created ON images(user_id, created_at DESC) WHERE deleted_at IS NULL;
```

Worker-specific partial indexes avoid full-table scans in background loops:
```sql
-- AI worker: unprocessed images
CREATE INDEX idx_images_embedding_status ON images(embedding_generated_at)
    WHERE embedding_generated_at IS NULL AND deleted_at IS NULL;

-- Replication worker: unsynced images
CREATE INDEX idx_images_need_sync ON images(created_at)
    WHERE p2p_synced_at IS NULL;
```

## Migrations

All migrations are idempotent (`ADD COLUMN IF NOT EXISTS`, `CREATE INDEX IF NOT EXISTS`). They run automatically on every server startup via `db::run_migrations`.

| File | What it adds |
|------|-------------|
| `001_fix_partial_indexes_deleted_at.sql` | Rebuilds partial indexes on `deleted_at` after adding the column post-launch |
| `002_add_duplicate_pairs.sql` | `image_duplicate_pairs` table for near-duplicate detection |
| `003_add_orientation_column.sql` | `orientation` on `images` for EXIF rotation |
| `004_multi_tenancy.sql` | `user_id` FKs across all tables; admin/user roles |
| `005_add_segmented_sharding.sql` | `p2p_segment_count` + `p2p_segment_enc_sizes` for large-file P2P support |

To apply manually:
```bash
psql "$DATABASE_URL" -f db/migrations/005_add_segmented_sharding.sql
```

## Fresh Setup

`db/init.sql` creates the full schema from scratch including all idempotent `ALTER TABLE` statements from past migrations, so a new container doesn't need to run migration files separately.
