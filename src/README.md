# Backend Core (`src/`)

## Purpose
Core server implementation for Reminisce, containing the Actix-web HTTP API server, background worker tasks, database pool wrappers, and media processing utilities.

## Architecture & Startup Sequence
Startup is orchestrated by `lib.rs::run_server`:
1. Load configuration (`config.rs`) & initialize OpenTelemetry (`telemetry.rs`).
2. Initialize database pools (`db.rs` -> `MainDbPool` and `GeotaggingDbPool`).
3. Run pending database migrations automatically.
4. Initialize P2P node context if P2P mode is enabled (`np2p`).
5. Spawn background worker loops as Tokio tasks (`*_worker.rs`).
6. Bind and run Actix HTTP server (`services/`) with the configured middleware stack.

```
main.rs -> lib.rs::run_server ──┬──▶ DB Pools & Migrations
                                ├──▶ P2P Node Context
                                ├──▶ Tokio Workers (AI, Verification, Replication, Audit, Rebalance, Duplicates)
                                └──▶ Actix HTTP Server (Middleware: Tracing, CORS, Auth, OpenAPI)
```

## Key Patterns & Conventions
- **Dependency Injection (DI)**: State is passed to Actix handlers using custom newtypes around `deadpool_postgres::Pool`: `MainDbPool` and `GeotaggingDbPool`.
- **Raw SQL over ORM**: Queries are written in raw SQL with `tokio-postgres` and `deadpool-postgres`. Dynamic queries use `query_builder.rs`.
- **Worker Pattern**: Background workers implement `run_worker_loop` with adaptive backoff, monitoring `CancellationToken`. Worker functions return `Result<bool, Error>` where `Ok(true)` indicates work was performed (prompting immediate re-loop without backoff).
- **Error Handling**: Handlers log errors at appropriate tracing levels and map internal errors to HTTP responses (JSON error body + appropriate status code).

## Invariants & Gotchas
- **Database Pooling**: Always extract DB pools using the `MainDbPool` / `GeotaggingDbPool` newtypes rather than raw pool types to maintain type safety across web handlers.
- **Worker Cancellation**: Every worker loop must respect the `CancellationToken` to ensure clean server shutdown without orphan tasks.
- **Backoff Logic**: Workers must sleep/backoff on `Ok(false)` (no work found) or `Err` to prevent CPU spinning.

## Key Files
- [lib.rs](file:///Users/ldr/work/reminisce/src/lib.rs): Server setup, DI state configuration, worker initialization, route table.
- [db.rs](file:///Users/ldr/work/reminisce/src/db.rs): Database pool wrappers (`MainDbPool`, `GeotaggingDbPool`).
- [query_builder.rs](file:///Users/ldr/work/reminisce/src/query_builder.rs): Dynamic SQL generator for media queries.
- [media_utils.rs](file:///Users/ldr/work/reminisce/src/media_utils.rs): Thumbnail generation, EXIF metadata extraction.
- [geo_utils.rs](file:///Users/ldr/work/reminisce/src/geo_utils.rs): Geocoding & reverse geocoding via PostGIS / geotagging DB.
