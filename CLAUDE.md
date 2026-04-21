# Reminisce — Development Notes

## Documentation Convention

When modifying the codebase, keep the docs layer that owns that area current:

- **Adding or removing an HTTP handler**: add a `#[utoipa::path(...)]` annotation and register it in the `#[openapi(paths(...))]` macro in `src/lib.rs`. No separate API doc file needed — Swagger UI at `/swagger-ui/` is the API reference.
- **Changing a worker, p2p_restore, or coordinator**: check whether `docs/p2p-backup.md` or `docs/architecture.md` needs updating.
- **Adding or removing a DB column or table**: update `docs/database.md`.
- **Changing deployment config, Docker setup, or first-run flow**: update `docs/deployment.md`.
- **Non-obvious module logic**: add or update the `//!` doc block at the top of the source file.

## Tests

All integration tests use an ephemeral Postgres instance via `setup_test_database_with_instance()`. Tests that share a DB must use `#[serial]` from `serial_test` to avoid conflicts.

Run the full suite:
```bash
cargo test
```

Run a specific test file:
```bash
cargo test --test shard_rebalance_worker_test
```
