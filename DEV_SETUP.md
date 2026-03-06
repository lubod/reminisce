# Local Development Environment Setup

This guide will help you set up a complete local development environment where you can quickly iterate on code changes.

## Architecture

**Hybrid Development Setup:**
- **Infrastructure** (PostgreSQL, AI services, Nginx, Storage Nodes) runs in Docker containers.
- **Reminisce server** (Rust) runs locally via `cargo run` for fast iteration and debugging.
- **Client** (React/Vite) runs locally via `npm run dev` for hot module replacement (HMR).

This approach provides fast code reloading, easy debugging with IDE integration, and handles heavy ML/GIS dependencies via Docker.

---

## Prerequisites

### 1. Install Rust & Build Tools
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# Install mold for faster linking (highly recommended)
sudo apt-get update && sudo apt-get install -y mold clang
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

### Step 1: Initial Configuration
Copy the example configuration and fill in your values:
```bash
cp config-dev.yaml.example config-dev.yaml
```

### Step 2: Build Images (Optimized)
The project uses BuildKit cache mounts and `cargo-chef` to make builds extremely fast.
```bash
# Build all Docker images
./dev docker-build
```

### Step 3: Start Development Infrastructure
```bash
# Start core services (databases, AI, P2P storage, nginx)
./dev start
```

### Step 4: Run the Application

#### A. Hybrid Mode (Recommended for Coding)
Run the infrastructure in Docker, but the app code locally.

```bash
# Terminal 1: Backend (with auto-reload)
./dev watch

# Terminal 2: Frontend (Vite HMR)
./dev client
```

#### B. Fullstack Docker Mode (For testing final product)
Run every single component inside Docker containers.
```bash
./dev fullstack
```

---

## Development Commands Reference

The `./dev` script is your primary tool for development:

### Infrastructure Management
| Command | Description |
|---------|-------------|
| `./dev up`, `start` | Start core infra (DBs, AI, Nginx, 2 Shard nodes) |
| `./dev down`, `stop`| Stop all containers and cleanup projects |
| `./dev recreate`    | Force stop and recreate infra containers |
| `./dev ps`          | Show status of all running containers |
| `./dev logs [svc]`  | Follow logs of infra containers |
| `./dev shell <svc>` | Get an interactive shell into a container |

### Local Development
| Command | Description |
|---------|-------------|
| `./dev run`         | Build and run Rust server locally |
| `./dev watch`       | Auto-rebuild and restart Rust server locally |
| `./dev client`      | Start Vite dev server for React |
| `./dev test`        | Run all Rust tests (uses dev DBs) |

### Docker & Deployment
| Command | Description |
|---------|-------------|
| `./dev docker-build`   | Build all Docker images (optimized) |
| `./dev docker-rebuild` | Rebuild images from scratch (no-cache) |
| `./dev docker-push`    | Push built images to the registry |
| `./dev fullstack`      | Start everything (server, client, all nodes) in Docker |
| `./dev fullstack-recreate` | Force recreate all containers in Docker |

### Maintenance
| Command | Description |
|---------|-------------|
| `./dev clean`        | Remove local Rust build artifacts |
| `./dev clean-docker` | **Destructive**: Remove all project containers AND volumes |
| `./dev restore <file>`| Restore database from a backup SQL file |
| `./dev start-obs`    | Start observability stack (Grafana/Prometheus) |

---

## Access Points

- **Frontend (Vite HMR):** http://localhost:5173
- **Frontend (Production UI):** https://localhost:28444 (via Nginx)
- **API (Direct):** http://localhost:8080
- **Swagger UI:** http://localhost:8080/swagger-ui/
- **Grafana:** http://localhost:3000

---

## GPU Acceleration (Automatic)

The AI service automatically detects and uses available GPUs (Intel, AMD, or NVIDIA) via `/dev/dri` access. It falls back to CPU if no GPU is found.

**Check GPU status:**
```bash
docker logs reminisce-dev-ai-server | grep -i "device\|gpu"
```

---

## Troubleshooting

### Caching Issues
If a Docker build seems stuck or corrupted, you can bypass the cache:
```bash
./dev docker-rebuild
```

### Resetting Everything
If you want to start from a completely clean slate (including deleting all uploaded images and database data):
```bash
./dev clean-docker
```
