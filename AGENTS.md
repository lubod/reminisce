# Developer Guidelines (AGENTS.md)

This file contains compilation, test, lint, and build instructions for AI-assisted coding and verification in the Reminisce project.

## Verification Commands

### Core Workspace Check
To verify that all crates compile without type or borrow checker errors:
```bash
cargo check --workspace --all-targets
```

### Run Unit Tests
To run non-database unit tests (useful when Docker/PostgreSQL is offline):
```bash
cargo test --workspace --lib -- --skip test_utils
```

To run all unit and integration tests (requires Docker and PostgreSQL database running):
```bash
cargo test --workspace
```

### Linting and Code Quality
To run Clippy lints across the workspace:
```bash
cargo clippy --workspace --all-targets -- -D warnings
```

### Pre-Push Verification Hook
A repo-checked-in git hook runs the full local verification suite (cargo check, clippy, unit tests, client build) before every `git push`. Install it once per clone:
```bash
./dev install-hooks        # or: git config core.hooksPath githooks
```
The hook lives at [githooks/pre-push](file:///Users/ldr/work/reminisce/githooks/pre-push). Full DB integration tests are NOT run by the hook (they need Docker) — run them separately with `./dev test`. Bypass in an emergency with `git push --no-verify`.

---

## Client Application (Vite / React)

All client/frontend development commands must be run within the `client/` directory.

### Install Dependencies
```bash
npm install
```

### Start Development Server
```bash
npm run dev
```

### Production Build Check
```bash
npm run build
```

---

## Database Schema & Migrations

The database initialization and composite indexes are defined in:
- [db/init.sql](file:///Users/ldr/work/reminisce/db/init.sql)

---

## Documentation Maintenance

- When adding, removing, or relocating code modules, HTTP handlers, or state stores, update the corresponding `README.md` file in that directory to keep the architecture map and invariants accurate.

