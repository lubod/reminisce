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
