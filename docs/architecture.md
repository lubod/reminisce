# System Architecture & Technical Deep-Dive

## System Overview

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│                             CLIENT APPLICATIONS                                  │
│   React 19 Web SPA (Vite/MobX)    •    Android Mobile App (Auto Background Sync) │
└────────────────────────────────────────┬─────────────────────────────────────────┘
                                         │ HTTPS / Bearer Auth
                                         ▼
┌──────────────────────────────────────────────────────────────────────────────────┐
│                         BACKEND CORE (`reminisce` - Rust)                        │
│  Actix-web HTTP REST API   •   OpenAPI / Swagger   •   Multi-Tenancy Claims      │
│  7 Background Tokio Workers (AI, Replication, Audit, Rebalance, Dupes, Verify, DB Backup)  │
└──────────────┬─────────────────────────┬─────────────────────────┬───────────────┘
               │                         │                         │
               ▼                         ▼                         ▼
┌──────────────────────────┐ ┌──────────────────────┐ ┌──────────────────────────┐
│  DATABASE & METADATA     │ │  AI INFERENCE ENGINE │ │   P2P STORAGE MESH       │
│  PostgreSQL 16           │ │  Python (Flask+gRPC) │ │   `np2p` Nodes (Raspberry│
│  ├── pgvector (Embeddings│ │  ├── SigLIP2 (Vector)│ │    Pi / Dedicated Disks) │
│  └── PostGIS (Geocoding) │ │  ├── SmolVLM (Desc)  │ │   ├── 3/5 Reed-Solomon   │
│                          │ │  └── InsightFace     │ │   └── ChaCha20-Poly1305  │
└──────────────────────────┘ └──────────────────────┘ └──────────────────────────┘
                                                                   ▲
                                                                   │ QUIC Tunnel
                                                     ┌─────────────┴────────────┐
                                                     │  COORDINATOR (VPS Relay) │
                                                     │  NAT Traversal / Tunnel  │
                                                     └──────────────────────────┘
```

## Crates & Services

| Crate / Service | Path | Responsibility |
|-----------------|------|----------------|
| `reminisce` | `/` | REST API, background worker orchestration, multi-tenancy, dynamic query building |
| `np2p` | `np2p/` | P2P storage engine: QUIC transport, ChaCha20-Poly1305 encryption, 3/5 Reed-Solomon erasure coding, UDP LAN discovery |
| `coordinator` | `coordinator/` | VPS-hosted peer signaling registry + QUIC/TCP reverse tunnel relay for WAN traversal |
| `ai-service` | `ai/` | Python sidecar serving gRPC `:50051` for all inference (SigLIP2 embeddings, SmolVLM/Qwen captions, InsightFace detection, quality scoring, image enhancement) over shared models; HTTP `:8081` kept only for `/health` |

---

## Module Documentation Index

Co-located architecture and invariant guides are maintained alongside code modules:

- **Backend Core**: [src/README.md](file:///Users/ldr/work/reminisce/src/README.md) • [src/services/README.md](file:///Users/ldr/work/reminisce/src/services/README.md)
- **Client Application**: [client/README.md](file:///Users/ldr/work/reminisce/client/README.md) • [client/src/README.md](file:///Users/ldr/work/reminisce/client/src/README.md) • [client/src/stores/README.md](file:///Users/ldr/work/reminisce/client/src/stores/README.md) • [client/src/api/README.md](file:///Users/ldr/work/reminisce/client/src/api/README.md) • [client/src/components/README.md](file:///Users/ldr/work/reminisce/client/src/components/README.md)
- **P2P & Storage Crate**: [np2p/README.md](file:///Users/ldr/work/reminisce/np2p/README.md) • [np2p/src/network/README.md](file:///Users/ldr/work/reminisce/np2p/src/network/README.md) • [np2p/src/storage/README.md](file:///Users/ldr/work/reminisce/np2p/src/storage/README.md)
- **Coordinator Daemon**: [coordinator/README.md](file:///Users/ldr/work/reminisce/coordinator/README.md)
- **AI Inference Service**: [ai/README.md](file:///Users/ldr/work/reminisce/ai/README.md)

---

## 1. Backend API & Background Workers (`src/`)

### Server Initialization Flow (`lib.rs::run_server`)
1. **Config & Secret Validation**: Verifies `api_secret_key` strength ($\ge 32$ characters) to protect JWT HS512 signatures.
2. **Connection Pools**:
   - `MainDbPool`: Dedicated pool for web HTTP REST handlers.
   - `WorkerDbPool`: Separate connection pool (capped at 10) for background workers to prevent database connection starvation during bulk indexing.
   - `GeotaggingDbPool`: Separate connection pool for PostGIS location resolution.
3. **Automated Schema Migrations**: Runs idempotent database migrations on startup.
4. **Worker Spawning**: Spawns 7 Tokio tasks sharing a `tokio_util::sync::CancellationToken` for graceful shutdown on SIGINT/SIGTERM.

### Multi-Tenancy & Authentication
- **Declarative Auth (`Claims`)**: REST endpoints declare `claims: Claims` in their function signature. `Claims` implements `actix_web::FromRequest` to automatically validate `Authorization: Bearer <token>` headers.
- **Imperative Auth (`authenticate_request`)**: Used for raw media/thumbnail streaming endpoints where HTML `<img>` and `<video>` tags supply tokens via `?token=` query parameters.
- **Native form login (`POST /api/auth/user-login-form`)**: the web login form is a real `application/x-www-form-urlencoded` POST — the backend sets the same httpOnly session cookie and responds `302` to `/` (or `/login?error=1`). Unlike a JSON/XHR login this is a genuine form submission + navigation, which makes browser password managers offer to save the credentials.
- **Multi-Tenant Isolation**: Enforced at the SQL query layer using `WHERE user_id = $N` matching `claims.sub`.
- **SQL Injection Safeguards**: Dynamic table interpolations are checked against an explicit whitelist via `validate_table_name()` (`"images"`, `"videos"`, etc.). `query_builder.rs` formats all user inputs as positional SQL parameters (`$1`, `$2`, ...).

### Background Worker Architecture (`*_worker.rs`)
Workers run an adaptive loop with exponential backoff (`run_worker_loop`):
- Returning `Ok(true)` indicates work was performed (resets loop to `min_interval`).
- Returning `Ok(false)` or `Err` triggers exponential backoff up to `max_interval`.
- **Dynamic Resource Throttling**: `calculate_worker_concurrency()` monitors system CPU load average and GPU utilization (`get_gpu_load()`) to automatically scale worker batch sizes down during high host load.

| Worker | File | Responsibility & Pipeline |
|--------|------|---------------------------|
| **AI Worker** | `ai_worker.rs` | Fetches unindexed media (images + video keyframes), resizes via CPU thread pool (`web::block`), calls the AI service over gRPC (`:50051`), and saves SigLIP2 embeddings, SmolVLM descriptions, InsightFace clusters, and — for images with no EXIF — a BEiT orientation-classifier result stored into `images.orientation` / `images.orientation_detected_at`. |
| **Orientation Check UI** | `client/src/components/OrientationCheck.tsx` | Lists no-EXIF photos (`/api/image_thumbnails?no_exif=true`) in a review grid + lightbox so users can manually check AI-detected orientation. |
| **Verification Worker** | `verification_worker.rs` | Computes BLAKE3 checksums of local files on disk; flags `verification_status` to trigger restoration if files are corrupted or missing. |
| **Replication Worker** | `media_replication_worker.rs` | Uses Rendezvous Hashing to select top 5 storage nodes for unsynced files, encrypts payload (ChaCha20-Poly1305), encodes into 3/5 Reed-Solomon shards, and streams to nodes over QUIC. |
| **Audit Worker** | `p2p_audit_worker.rs` | Verifies shard health across storage nodes; if available shards drop below 5 (but $\ge 3$), downloads surviving shards, reconstructs the missing shards, and pushes them to new storage nodes. |
| **Shard Rebalance Worker** | `shard_rebalance_worker.rs` | Re-evaluates Rendezvous hash rankings when nodes join or leave the mesh and migrates shards to optimal nodes. |
| **Duplicates Worker** | `duplicate_worker.rs` | Computes cosine distance matrices on image embeddings via `pgvector` (`1 - (embedding <=> target)`) to group near-duplicate photo candidate pairs. |
| **DB Backup Worker** | `db_backup_worker.rs` | On a fixed interval (default 24h) runs `pg_dump -Fc`, encrypts + erasure-codes the dump into 3/5 shards, and pushes them to P2P nodes. Keeps the last N snapshots in a `db_backups` manifest and prunes older ones (deletes remote shards via `DeleteShardRequest`). |

---

## 2. P2P Storage Mesh & Networking (`np2p` & `coordinator`)

### Shard Encryption & Storage Pipeline

```
Raw Media Stream ──▶ ChaCha20-Poly1305 Encrypt ──▶ Reed-Solomon (3 Data + 2 Parity) ──▶ Shard Disk Layout
```

1. **ChaCha20-Poly1305 Encryption (`encryption.rs`)**:
   - **Deterministic Nonce Invariant**: Nonces are derived deterministically using $\text{BLAKE3}(\text{file\_hash} \parallel \text{segment})[0..12]$.
   - Re-encrypting the same file produces byte-for-byte identical ciphertexts, enabling independent audit workers to reconstruct missing shards without state desynchronization.
2. **3/5 Reed-Solomon Erasure Coding (`erasure.rs`)**:
   - Encrypted data is split into **3 Data Shards** ($k=3$) and **2 Parity Shards** ($m=2$).
   - Any **3 of 5 shards** are mathematically sufficient to recover the original file. Up to 2 storage nodes can fail simultaneously without data loss.
3. **Disk Engine & Safety Guards (`disk.rs`)**:
   - Content-addressed disk layout: `<storage_root>/shards/<hash[0..2]>/<hash[2..]>`.
   - Streaming uploads append to `.tmp` files and verify the complete BLAKE3 hash before atomic rename.
   - **100 MB Free Space Guard**: Storage nodes verify `statvfs` space before accepting new shards, rejecting uploads if available space falls below 100 MB.

### Peer Discovery & WAN Routing
- **LAN Discovery (`discovery.rs`)**: Storage nodes broadcast Ed25519-signed UDP packets on port 5066 every 10s. Peers maintain local registry entries with a 90-second TTL.
- **Rendezvous Hashing (HRW)**: Determines shard placement by ranking active nodes using $\text{BLAKE3}(\text{NodeID}_i \parallel \text{FileHash})$.
- **VPS Coordinator Relay (`coordinator`)**: VPS signaling daemon on port `:5055` (QUIC) and reverse tunnel relay on port `:8443` (TCP). Facilitates WAN NAT traversal without decrypting payload traffic.

---

## 3. AI & Multi-Modal Vector Search Architecture

### Transport: gRPC + HTTP
The backend talks to the AI service over **gRPC (`:50051`)** for all inference (image/text embeddings, descriptions, face detection, quality scoring, image enhancement) using binary protobuf — see `proto/ai_service.proto` and the Rust client `src/ai_client.rs` (a shared, lazily-connected `tonic` `Channel`). The **Flask HTTP server (`:8081`)** is kept only for `/health` (container healthchecks). Both interfaces are authenticated with the same API key (`x-api-key` gRPC metadata / `X-API-Key` or Bearer HTTP header).

### Model Inventory
- **SigLIP 2** (`google/siglip2-so400m-patch14-384`): Generates 1152-dimensional multi-modal vector embeddings for image-to-text and image-to-image semantic search.
- **SmolVLM-500M & Qwen2.5-VL-3B**: Generates visual scene descriptions (captions). Visual tokens are capped at 256 (`max_pixels=256*28*28`) for fast processing.
- **InsightFace** (`buffalo_l`): Extracts bounding boxes `[x, y, w, h]` and 512-dimensional face recognition vectors on CPU.

### Description Processing Resilience (`ai_worker.rs` / `ai_client.rs`)
Descriptions are the one AI path that can hard-crash the service: a ROCm fault (e.g. `HSA_STATUS_ERROR_EXCEPTION 0x1016` on the AMD iGPU) aborts the Python process mid-call. To survive that without a crash-loop:
- An in-progress marker `[__processing__]` is written **before** the gRPC call, plus `description_started_at`, so a row left mid-flight is parked, not re-selected.
- Each gRPC call has a **180s deadline** (`RPC_TIMEOUT`) so a hung (alive) inference can't wedge a worker semaphore permit forever.
- Mid-call transport failures (the process crashed on this specific image) increment `description_failed_attempts`; after `DESC_FAIL_CAP = 5` the row is permanently parked as `[skipped]`. A pure `gRPC connect failed` (the whole service is down) never counts — otherwise a weekend outage would bulk-skip the library.
- A periodic un-park sweep resets `[__processing__]` rows idle at the marker for >15 min (gated on `description_started_at`, not ingestion time), parking any that already exhausted the retry cap.
- The dashboard "AI Description In Progress" count (`stats.rs` `images_description_pending`) mirrors the worker's selection predicate (verified, non-SVG, description null/processing) so it can't show a permanent stale count.

### Location/Radius Search
Location (`lat`/`lon`/radius) filters apply to **images only** — `videos` have no `location` column. `text_search.rs`, `embedding.rs`, and `map.rs` all guard the location SQL with an images-only (`alias == "i"`) condition and emit `NULL` distance for the video arm, so a radius query doesn't 500 against video rows.

### Video Semantic Search
The AI worker extracts keyframes from each video with ffmpeg (every ~5s, capped), embeds each keyframe via `EmbedImage`, stores per-keyframe vectors in `video_keyframes` (with `timestamp_secs`), and writes the normalized centroid into `videos.embedding`. Semantic search in `embedding.rs` then runs per-branch KNN over both `images` and `videos` and merges the results.

### `pgvector` Database Indexing (`db/init.sql`)
- **Image Embeddings**: Stored in `images.embedding vector(1152)` indexed using HNSW:
  ```sql
  CREATE INDEX idx_images_embedding_hnsw ON images
  USING hnsw (embedding vector_cosine_ops)
  WITH (m = 16, ef_construction = 64);
  ```
- **Semantic Vector Queries (`embedding.rs`)**:
  Converts text queries into 1152-dim vectors ($\vec{T}$) and executes cosine distance search:
  $$\text{Similarity Score} = 1.0 - (\text{embedding} \mathbin{\langle\Rightarrow\rangle} \vec{T})$$
- **Facial Recognition & Clustering (`face_detection.rs` & `person.rs`)**:
  InsightFace vectors are matched against `persons.representative_embedding` (threshold distance $\le 0.35$). Centroids update dynamically:
  $$\vec{C}_{\text{new}} = \frac{N \cdot \vec{C}_{\text{old}} + \vec{f}}{N + 1}$$
- **Hybrid Search**: Combines `pgvector` HNSW semantic queries with PostgreSQL GIN full-text search (`to_tsvector('english', description || name)`).

---

## 4. Frontend & Mobile Client Architecture (`client/` & `android_app/`)

### Web Client (`client/`)
- **Tech Stack**: React 19 + TypeScript + Vite 5 + MobX + Tailwind CSS 4.
- **MobX RootStore Pattern**: Single `RootStore` manages 8 domain stores (`AuthStore`, `MediaStore`, `PersonStore`, `DuplicatesStore`, `TrashStore`, `UIStore`, `StatsStore`, `LabelStore`).
- **Store Conventions**: Constructors invoke `makeAutoObservable(this)`. Post-await state mutations are wrapped in `runInAction()`. Components use `observer()` HOC wrappers for reactive re-rendering.
- **Automatic Debouncing**: `MediaStore` uses MobX `reaction()` to debounce search queries and filter state changes automatically (300ms delay).
- **Dual-Token Auth**:
  - Headers (`Authorization: Bearer <token>`) attached automatically via Axios request interceptor for JSON API endpoints.
  - Query parameters (`?token=${imageToken}`) appended to media URL sources for HTML `<img>` and `<video>` elements.
  - 401 Response Interceptor automatically clears local storage tokens and redirects unauthenticated users to `/login`.

### Android Application (`android_app/`)
- **Background Sync (`WorkManager`)**: Scans Android `MediaStore` for new media and automatically uploads them to the home server when connected to Wi-Fi and charging.
- **QR Pairing**: Scans QR code generated by `AndroidConnectionQR.tsx` in the web interface to pair server host URL and API secrets.
- **Zero-Bandwidth Deduplication**: Computes local BLAKE3 hash prior to upload and checks `POST /api/upload/check-exists-batch`, skipping transfers for media already backed up.

### Observability & System Diagnostics

The Admin Dashboard provides real-time telemetry and diagnostics:

- **Logs & Errors Viewer (`ErrorsViewer.tsx`)**: Smart deduplication and frequency clustering for repeating errors (`12× occurrences`), target breakdown, severity filtering (`ALL`, `ERROR`, `WARN`, `PANIC`), text search, and 1-click clipboard copy.
- **Metrics Collector (`metrics_collector.rs`)**: In-memory and PostgreSQL persistent rolling metrics sampler capturing CPU/memory/disk, database pool utilization, per-hour AI throughput rates, and p95 request latencies.
- **Automated Health Alerts**: Real-time evaluation of DB connection health, disk thresholds, and worker loop backlogs.
