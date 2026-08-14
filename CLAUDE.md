# Reminisce — Development Notes

## Documentation Convention

When modifying the codebase, keep the docs layer that owns that area current:

- **Adding or removing an HTTP handler**: add a `#[utoipa::path(...)]` annotation and register it in the `#[openapi(paths(...))]` macro in `src/lib.rs`. No separate API doc file needed — Swagger UI at `/swagger-ui/` is the API reference.
- **Changing a worker, p2p_restore, or coordinator**: check whether `docs/p2p-backup.md` or `docs/architecture.md` needs updating.
- **Adding or removing a DB column or table**: update `docs/database.md`.
- **Changing deployment config, Docker setup, or first-run flow**: update `docs/deployment.md`.
- **Non-obvious module logic**: add or update the `//!` doc block at the top of the source file.

## Tests & Code Coverage

All integration tests use an ephemeral Postgres instance via `setup_test_database_with_instance()`. Tests that share a DB must use `#[serial]` from `serial_test` to avoid conflicts.

Run the full suite:
```bash
cargo test
# or via dev runner:
./dev test
```

Run a specific test file:
```bash
cargo test --test shard_rebalance_worker_test
cargo test --test observability_test
cargo test --test metrics_collector_test
```

### Coverage Quality Gates

Code coverage is strictly enforced during deployments via `/home/ldr/deploy.sh`:

- **Backend Line Coverage (`src/`)**: Gate: **$\ge 58\%$** (Current: **$62.51\%$** lines, **$59.59\%$** regions).
  ```bash
  ./scripts/coverage-backend.sh 58
  ```
- **P2P Storage Crate (`np2p/`)**: Gate: **$\ge 66\%$** (Current: **$79.42\%$** lines, **$79.68\%$** regions).
  ```bash
  ./scripts/coverage-np2p.sh 66
  ```
- **Client Web Suite (`client/`)**: Vitest unit & store tests (183 tests, >94% store coverage):
  ```bash
  cd client && npm test -- --run
  ```
