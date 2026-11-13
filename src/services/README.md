# Actix HTTP Services (`src/services/`)

## Purpose
HTTP API handlers grouped by feature domain. All REST endpoints exposed by Reminisce are defined in this directory.

## Handler & Route Conventions
- **Uniform Handler Pattern**: Handlers are standard Actix-web async functions annotated with HTTP method macros (e.g. `#[get("/path")]`) and `#[utoipa::path(...)]` for OpenAPI documentation.
- **DTOs & OpenAPI**: Request/response DTO structs derive `serde::Serialize`/`Deserialize` and `utoipa::ToSchema`.
- **Authentication**:
  - Declarative: Add `claims: Claims` parameter to handler signature (`Claims` implements `FromRequest`).
  - Imperative: Use `auth_utils::authenticate_request(&req, &config)` for custom stream/websocket/query-param token checks.
  - Admin Check: Handlers needing admin privileges check `claims.role != "admin"` inline (returning 403 Forbidden if not admin).
- **Route Registration**: New service modules must be registered in `lib.rs` under `HttpServer::new(...)` app configuration and added to the `ApiDoc` struct in `lib.rs` for Swagger UI generation.

## Service Inventory (27 Modules)

| Module | Role |
|--------|------|
| [ai_settings.rs](file:///Users/ldr/work/reminisce/src/services/ai_settings.rs) | Manage AI processing configuration and model parameters |
| [auth.rs](file:///Users/ldr/work/reminisce/src/services/auth.rs) | User authentication, login, and revocable refresh-token rotation/logout |
| [duplicates.rs](file:///Users/ldr/work/reminisce/src/services/duplicates.rs) | Duplicate media listing, review, and resolution |
| [embedding.rs](file:///Users/ldr/work/reminisce/src/services/embedding.rs) | Semantic vector search endpoints (per-branch KNN over images + videos; obtains query vectors via gRPC) |
| [existence_check.rs](file:///Users/ldr/work/reminisce/src/services/existence_check.rs) | Check existing hashes before upload to skip duplicates |
| [face_detection.rs](file:///Users/ldr/work/reminisce/src/services/face_detection.rs) | Face detection client lib (gRPC `DetectFaces`) — bounding boxes + 512-dim embeddings |
| [geocoding.rs](file:///Users/ldr/work/reminisce/src/services/geocoding.rs) | Reverse geocoding lookup and location data |
| [geodb_stats.rs](file:///Users/ldr/work/reminisce/src/services/geodb_stats.rs) | Offline reverse-geocoding DB status and stats |
| [health.rs](file:///Users/ldr/work/reminisce/src/services/health.rs) | Health check & readiness probes (`/health`, `/ready`) |
| [import_dir.rs](file:///Users/ldr/work/reminisce/src/services/import_dir.rs) | Server-side directory bulk import triggered via API |
| [ingest.rs](file:///Users/ldr/work/reminisce/src/services/ingest.rs) | Media ingestion pipeline status and triggering |
| [label.rs](file:///Users/ldr/work/reminisce/src/services/label.rs) | Tagging, custom labels, and media categorization |
| [maintenance.rs](file:///Users/ldr/work/reminisce/src/services/maintenance.rs) | Maintenance backfill helpers (repair original pixel dimensions) |
| [map.rs](file:///Users/ldr/work/reminisce/src/services/map.rs) | Paginated geotagged media points for map clustering (`/map/media`) |
| [media.rs](file:///Users/ldr/work/reminisce/src/services/media.rs) | Core media gallery CRUD, pagination, streaming, deletion |
| [observability.rs](file:///Users/ldr/work/reminisce/src/services/observability.rs) | In-app observability API: logs, alerts, errors, and AI GPU status (`/admin/gpu`) |
| [p2p_restore.rs](file:///Users/ldr/work/reminisce/src/services/p2p_restore.rs) | Trigger p2p node network restore for missing media |
| [p2p_status.rs](file:///Users/ldr/work/reminisce/src/services/p2p_status.rs) | P2P network topology, node status, shard stats, Android QR connection info (`local_ip`, `tailscale_url`, `tunnel_url`) |
| [person.rs](file:///Users/ldr/work/reminisce/src/services/person.rs) | Person profile management, face clustering & naming |
| [pool_stats.rs](file:///Users/ldr/work/reminisce/src/services/pool_stats.rs) | DB connection pool metrics and active pool telemetry |
| [quality.rs](file:///Users/ldr/work/reminisce/src/services/quality.rs) | Media quality score assessment queries |
| [stats.rs](file:///Users/ldr/work/reminisce/src/services/stats.rs) | Dashboard system overview statistics and media counts |
| [system_stats.rs](file:///Users/ldr/work/reminisce/src/services/system_stats.rs) | System hardware, storage usage, and host diagnostic metrics |
| [text_search.rs](file:///Users/ldr/work/reminisce/src/services/text_search.rs) | Full-text and metadata search queries |
| [thumbnail.rs](file:///Users/ldr/work/reminisce/src/services/thumbnail.rs) | Image thumbnail and video preview generation/delivery |
| [upload.rs](file:///Users/ldr/work/reminisce/src/services/upload.rs) | Direct media file upload handler (multipart forms) |
| [user_management.rs](file:///Users/ldr/work/reminisce/src/services/user_management.rs) | Admin user creation, role assignment, user listing |

## Invariants & Gotchas
- **Multi-Tenancy Enforced**: Every query filtering media or user assets MUST restrict results by `user_id` from `Claims` unless explicitly executing an admin cross-tenant inspection endpoint.
- **OpenAPI Schema Updates**: Any changes to request/response DTO structures or route signatures MUST be updated in the `#[utoipa::path(...)]` attribute to keep Swagger UI in sync.
- **Media path safety**: `upload.rs` metadata handlers (`/upload/image/metadata`, `/upload/video/metadata`) reject hashes that aren't 64 lowercase hex and unsafe file extensions. Serving handlers must resolve files through `media_utils::safe_resolve_content_path` (canonicalize + prefix check) — never interpolate raw user input into filesystem paths.
- **Auth-Note**: `user-login`/`setup`/`/auth/me` set an `HttpOnly` session cookie (`access_token`); `Claims` accepts header, cookie, or `?token=` so native clients keep working. The web client now relies on the cookie only (no localStorage/query-param tokens).
