# Local Development Environment Setup

This guide will help you set up a complete local development environment where you can quickly iterate on code changes.

## Architecture

**Hybrid Development Setup:**
- **Infrastructure** (PostgreSQL, AI services, Nginx) runs in Docker containers.
- **Reminisce server** (Rust) runs locally via `cargo run` for fast iteration and debugging.
- **Client** (React/Vite) runs locally via `npm run dev` for hot module replacement (HMR).

This approach provides fast code reloading, easy debugging with IDE integration, and handles heavy ML/GIS dependencies via Docker.

---

## Prerequisites

### 1. Install Rust & Build Tools
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# Install mold for faster linking (used in Docker builds)
sudo apt-get install -y mold clang
```

### 2. Install Node.js (v20 or later)
```bash
# Using nvm (recommended)
curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.0/install.sh | bash
nvm install 20
```

### 3. Install Docker & Buildx
Ensure you have the BuildKit plugin installed for optimized builds:
```bash
sudo apt-get update && sudo apt-get install -y docker-buildx
```

---

## Step-by-Step Setup

### Step 1: Build Images (Optimized)

The project uses BuildKit cache mounts and `cargo-chef` to make builds extremely fast.

```bash
# Build all Docker images (Postgres, AI, Backend, Client)
./dev docker-build
```

### Step 2: Start Development Infrastructure

```bash
# Start infrastructure (databases, AI, P2P nodes, nginx)
./dev start
```

### Step 3: Run the Application

#### A. Hybrid Mode (Recommended for Coding)
Run the infrastructure in Docker, but the app code locally.

```bash
# Terminal 1: Backend
./dev watch

# Terminal 2: Frontend
./dev client
```

#### B. Fullstack Docker Mode (For Testing)
Run every component inside Docker containers (useful for mobile app testing).

```bash
./dev fullstack
```

---

## Development Commands

The `./dev` script is your primary tool for development:

| Command | Description |
|---------|-------------|
| `./dev start` | Start DBs, AI, and Nginx infrastructure |
| `./dev stop` | Stop all containers |
| `./dev run` | Build and run Rust server locally |
| `./dev watch` | Auto-rebuild and restart Rust server locally |
| `./dev client` | Start Vite dev server for React |
| `./dev test` | Run all Rust tests (uses dev DBs) |
| `./dev docker-build` | Build all Docker images (optimized) |
| `./dev fullstack` | Start everything in Docker |
| `./dev clean` | Remove local build artifacts |
| `./dev start-obs` | Start observability stack (Grafana/Prometheus) |

---

## Access Points

- **Frontend (Vite HMR):** http://localhost:5173
- **Frontend (via Nginx):** https://localhost:28444 (Self-signed)
- **API (Direct):** http://localhost:8080
- **Swagger UI:** http://localhost:8080/swagger-ui/
- **Grafana:** http://localhost:3000

---

## GPU Acceleration (Automatic)

The AI service automatically detects and uses available GPUs (Intel, AMD, or NVIDIA) via `/dev/dri` access. It falls back to CPU if no GPU is found.

**Check GPU status:**
```bash
docker logs reminisce-dev-ai | grep -i "device\|gpu"
```

---

## Troubleshooting

### Caching Issues
If a Docker build seems stuck or corrupted, you can bypass the cache:
```bash
./dev docker-build --no-cache
```

### Port Conflicts
If port 8080 or 5432 is already in use:
```bash
lsof -i :8080
kill -9 <PID>
```
