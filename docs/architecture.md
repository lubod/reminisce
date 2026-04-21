# Architecture

## System Overview

```
┌──────────────────┐         ┌──────────────────────────────────────┐
│  Android App     │──HTTPS──▶│          Home Server                 │
└──────────────────┘         │  reminisce binary (Actix-web/Rust)   │
                             │  PostgreSQL + pgvector + PostGIS      │
┌──────────────────┐         │  AI service (Python/Flask)            │
│  React Client    │──HTTPS──▶│    SigLIP2 • SmolVLM • InsightFace   │
│  (browser)       │         └───────────────┬──────────────────────┘
└──────────────────┘                         │ QUIC / ChaCha20Poly1305
                                             │
                    ┌────────────────────────┼────────────────────────┐
                    │                        │                        │
           ┌────────▼────────┐    ┌──────────▼──────┐    ┌───────────▼─────┐
           │   Pi Node 1     │    │   Pi Node 2     │    │   Pi Node 3+    │
           │   np2p daemon   │    │   np2p daemon   │    │   np2p daemon   │
           │   local shards  │    │   local shards  │    │   local shards  │
           └─────────────────┘    └─────────────────┘    └─────────────────┘
                    ▲                        ▲
                    │   QUIC tunnel / relay   │
           ┌────────┴────────────────────────┴────────┐
           │         Coordinator (VPS)                │
           │  peer registry • relay • tunnel          │
           └──────────────────────────────────────────┘
                             ▲
                    Android connects via coordinator
                    when not on LAN
```

## Crates

| Crate | Path | Responsibility |
|-------|------|----------------|
| `reminisce` | `/` | REST API, workers, business logic |
| `np2p` | `np2p/` | P2P networking: QUIC transport, encryption, erasure coding, peer discovery |
| `coordinator` | `coordinator/` | VPS-hosted peer registry + QUIC relay/tunnel for WAN connectivity |

## Source Layout (`src/`)

| File / Dir | Purpose |
|------------|---------|
| `main.rs` | Binary entry point — loads config, calls `run_server` |
| `lib.rs` | Server setup: route registration, worker spawning, OpenAPI spec |
| `config.rs` | `Config` struct loaded from YAML |
| `db.rs` | Deadpool connection pool wrappers (`MainDbPool`, `GeotaggingDbPool`) |
| `services/` | One file per HTTP handler group |
| `*_worker.rs` | Background Tokio tasks (see Workers section) |
| `p2p_restore.rs` | P2P restore core logic (shared by HTTP handler and CLI binary) |
| `media_utils.rs` | EXIF extraction, thumbnail generation, geo parsing |
| `query_builder.rs` | Dynamic SQL for media gallery queries (filters, pagination, sorting) |
| `metrics.rs` | Prometheus counters/gauges exposed at `GET /metrics` |
| `telemetry.rs` | OpenTelemetry + tracing-subscriber initialization |

## Workers

All workers run as Tokio tasks spawned in `lib.rs::run_server`. Each runs an infinite loop with adaptive backoff.

| Worker | File | What it does |
|--------|------|--------------|
| AI worker | `ai_worker.rs` | Generates descriptions (SmolVLM) and embeddings (SigLIP2) for unprocessed media |
| Verification | `verification_worker.rs` | Periodically re-hashes files on disk; sets `verification_status` |
| Replication | `media_replication_worker.rs` | Rendezvous-hashes each unsynced file to 5 nodes; encrypts and uploads shards |
| Audit | `p2p_audit_worker.rs` | Checks every synced file has ≥3 shards; repairs missing shards |
| Rebalance | `shard_rebalance_worker.rs` | Migrates shards to their ideal nodes when topology changes |
| Duplicates | `duplicate_worker.rs` | Finds near-duplicate images using cosine similarity on embeddings |

## Multi-Tenancy

Every media row has a `user_id UUID` FK to `users`. All queries include `WHERE user_id = $N` enforced by the service layer — one user's data is never visible to another. Admin users can see all users via `GET /api/users` but not their media.

## Authentication

JWT HS512 tokens signed with `api_secret_key` from config. The `Claims` struct implements `actix_web::FromRequest` — handlers that declare `claims: Claims` in their signature automatically require a valid token (from `Authorization: Bearer <token>` header or `?token=` query param).

## OpenAPI / Swagger

The full API spec is available at runtime:
- Swagger UI: `http://localhost:8080/swagger-ui/`
- Raw JSON: `http://localhost:8080/api-doc/openapi.json`
