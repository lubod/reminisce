# Local Development Environment Setup

This guide will help you set up a complete local development environment where you can quickly iterate on code changes.

## Architecture

**Hybrid Development Setup:**
- **Databases** (Custom PostgreSQL with PostGIS + pgvector, Geotagging) run in Docker containers
- **AI services** (AI service (SigLIP2 + SmolVLM + InsightFace)) run in Docker containers
- **Nginx** with self-signed HTTPS runs in Docker container
- **Reminisce server** (Rust) runs locally via `cargo run` for fast iteration
- **Client** (React/Vite) runs locally via `npm run dev` for hot module replacement

This approach gives you:
- ✅ Fast code reloading (no Docker rebuilds)
- ✅ Easy debugging with IDE integration
- ✅ Simple database setup (no PostGIS installation hassle)
- ✅ AI service isolated in Docker (no ML library conflicts)
- ✅ HTTPS support via nginx with self-signed certificates
- ✅ Full access to logs and debugging tools

---

## Prerequisites

### 1. Install Rust
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

### 2. Install Node.js (v18 or later)
```bash
# Using nvm (recommended)
curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.0/install.sh | bash
nvm install 18
nvm use 18
```

### 3. Install Docker & Docker Compose
Already installed ✅

---

## Step-by-Step Setup

### Step 1: Build Images (First Time Only)

```bash
# Build all Docker images including the custom PostgreSQL image
docker compose -f docker-compose-build.yml build

# This builds:
# - PostgreSQL with PostGIS + pgvector
# - AI service
# - AI service (included above)
# - Geotagging database
# - Main application (optional, for Docker deployment)
```

### Step 2: Start Development Services (Databases + AI Services + Nginx)

```bash
# Start all services (databases, AI service, P2P nodes, nginx)
docker compose -f docker-compose-dev.yml up -d

# Or use the convenient start script
./dev-start.sh

# Verify all services are running
docker compose -f docker-compose-dev.yml ps

# Check database initialization (users table should be created)
docker exec -it reminisce-dev-db psql -U postgres -d reminisce_db -c "\dt"
```

You should see tables: `images`, `videos`, `users`

**Note:** AI services will download models on first startup:
- AI service: downloads SigLIP2, SmolVLM-500M, and InsightFace models on first start (~2-4GB total)

You can monitor progress with:
```bash
docker logs -f reminisce-dev-ai
docker logs -f reminisce-dev-face
```

Once ready, check the health endpoints:
```bash
curl http://localhost:8081/health  # AI service
curl http://localhost:8082/health  # Face
```

**Nginx with HTTPS:** Self-signed SSL certificates are automatically generated. Your browser will show a security warning - this is normal for development. Click "Advanced" and "Proceed to localhost" to continue.

Check nginx logs if needed:
```bash
docker logs reminisce-dev-nginx
```

### Step 3: Create Upload Directories

```bash
mkdir -p uploaded_images uploaded_videos
chmod 755 uploaded_images uploaded_videos
```

### Step 4: Run Reminisce Server (Rust)

```bash
# Build and run with dev config
RUST_LOG=info cargo run --bin reminisce_bin -- config-dev.yml

# Or for development with auto-reload (install cargo-watch first)
# cargo install cargo-watch
# cargo watch -x 'run --bin reminisce_bin -- config-dev.yml'
```

The server will start on `http://0.0.0.0:8080`

**Expected output:**
```
INFO Server starting up with config file
INFO DATABASE_URL: postgres://postgres:postgres@localhost:5432/reminisce_db
INFO Starting HTTP server on 0.0.0.0:8080
```

### Step 5: Run Client (React/Vite)

Open a **new terminal window**:

```bash
cd client

# Install dependencies (first time only)
npm install

# Start development server
npm run dev
```

**Access Options:**

1. **Via Nginx with HTTPS** (recommended): `https://localhost:28443`
   - Self-signed certificate (browser will warn - click "Advanced" → "Proceed")
   - Full production-like environment
   - API proxied to backend via nginx

2. **Direct Vite Dev Server**: `http://localhost:5173`
   - Fastest for frontend-only development
   - No HTTPS

**Vite dev server features:**
- Hot Module Replacement (HMR) - instant updates on file save
- API proxy configured to forward `/api/*` to `http://localhost:8080`
- TypeScript type checking

---

## Development Workflow

### Making Backend Changes (Rust)

1. Edit files in `src/`
2. Save the file
3. Cargo will automatically recompile
4. Restart the server (Ctrl+C, then `cargo run --bin reminisce_bin -- config-dev.yml`)
5. Test your changes

**Example: Testing new auth endpoints**
```bash
# Register a new user
curl -X POST http://localhost:8080/auth/register \
  -H "Content-Type: application/json" \
  -d '{
    "username": "testuser",
    "email": "test@example.com",
    "password": "testpass123"
  }'

# Login
curl -X POST http://localhost:8080/auth/user-login \
  -H "Content-Type: application/json" \
  -d '{
    "username": "testuser",
    "password": "testpass123",
    "device_id": "dev-machine"
  }'
```

### Making Frontend Changes (React)

1. Edit files in `client/src/`
2. Save the file
3. Browser will auto-reload with changes (HMR)
4. No manual refresh needed!

### Database Operations

```bash
# Connect to main database
docker exec -it reminisce-dev-db psql -U postgres -d reminisce_db

# Connect to geotagging database
docker exec -it reminisce-dev-geotagging psql -U postgres -d geotagging_db

# Useful SQL commands:
\dt                    # List tables
\d users              # Describe users table
SELECT * FROM users;  # View all users
\dx                   # List installed extensions (should show postgis, vector)

# Check vector embeddings
SELECT COUNT(*) FROM images WHERE embedding IS NOT NULL;
```

### AI Service Operations

The AI service provides image embeddings (SigLIP2), descriptions (SmolVLM-500M), and face detection (InsightFace).

```bash
# Check AI service health
curl http://localhost:8081/health

# View AI service logs
docker logs -f reminisce-dev-ai

# Rebuild AI service (after code changes)
docker compose -f docker-compose-dev.yml build ai-server
docker compose -f docker-compose-dev.yml up -d ai-server
docker compose -f docker-compose-dev.yml logs ai-server | grep "detected\|loaded\|device"
```

**Performance Notes:**
- GPU support is **enabled by default** (Intel, AMD, NVIDIA via `/dev/dri`)
- Automatically falls back to CPU if no GPU is available
- GPU is 5-10x faster than CPU (see "GPU Acceleration" section below)
- Model cache is persisted in Docker volumes `ai_dev_model_cache` and `face_dev_model_cache`





```bash

curl http://localhost:5000/health





docker compose -f docker-compose-dev.yml build ai-server
docker compose -f docker-compose-dev.yml up -d ai-server
```

**Performance Notes:**

- GPU support via Vulkan (Intel, AMD, NVIDIA via `/dev/dri`)
- Automatically falls back to CPU if no GPU is available
- CPU inference can be slow (30-120s per image)
- GPU inference is much faster (1-5s per image)

### Nginx Operations

Nginx provides HTTPS access to your development environment.

```bash
# View nginx logs
docker logs -f reminisce-dev-nginx

# Test HTTPS endpoint
curl -k https://localhost:28443

# Restart nginx (after config changes)
docker compose -f docker-compose-dev.yml restart nginx

# Regenerate SSL certificates
docker compose -f docker-compose-dev.yml down
docker volume rm reminisce_nginx_dev_ssl
docker compose -f docker-compose-dev.yml up -d
```

**Access Points:**
- **HTTPS**: https://localhost:28443 (main access point)
- **HTTP**: http://localhost:28080 (redirects to HTTPS)
- **Backend Direct**: http://localhost:8080 (without nginx)
- **Vite Direct**: http://localhost:5173 (without nginx)
- **AI Service**: http://localhost:8081


---
 
## P2P Configuration (Backups)

Reminisce uses a distributed storage model where encrypted shards of your files are stored across multiple nodes.
It is designed to work over stable overlay networks like **NetBird**, **Tailscale**, or **WireGuard**.

### Configuration
Define your storage nodes in `config.yaml` using their static overlay IPs:

```yaml
p2p_peers:
  - "100.x.x.x:5050"
  - "100.x.x.x:5051"
  - "100.x.x.x:5052"
  - "100.x.x.x:5053"
  - "100.x.x.x:5054"
```

**Requirements:**
- Each node must be running the `np2pd` daemon.
- Port `5050/UDP` (or custom) must be open between nodes on the overlay network.
 
---

## Fullstack Docker Setup (Recommended for Testing)

For end-to-end testing (web UI + Android app), use the fully dockerized setup where **everything** runs in Docker containers connected via NetBird mesh VPN.

### Prerequisites

1. A NetBird setup key from [your-netbird-server.example.com](https://your-netbird-server.example.com)
2. Docker images built: `docker compose -f docker-compose-build.yml build`

### Setup

```bash
# 1. Set your NetBird setup key
echo 'NETBIRD_SETUP_KEY=<your-key>' > .env

# 2. Start everything
docker compose -f docker-compose-dev.yml up -d

# 3. Verify all services are running
docker compose -f docker-compose-dev.yml ps

# 4. Check NetBird connectivity
docker exec netbird-client netbird status
```

### What's Running

| Service | Container | Access |
|---------|-----------|--------|
| NetBird VPN | `netbird-client` | Overlay IP (e.g. 100.x.x.x) |
| PostgreSQL | `reminisce-dev-db` | localhost:5432 |
| GeotaggingDB | `reminisce-dev-geotagging` | localhost:5435 |
| AI Server | `reminisce-dev-ai` | localhost:8081 |
| Reminisce Server | `reminisce-dev-server` | localhost:8080 (host network) |
| P2P Storage Nodes | `p2p-dev-node-1..5` | ports 5050-5054/udp |
| Nginx + Web UI | `reminisce-dev-nginx` | HTTP :28081, HTTPS :28444 |

### Access Points

- **Web UI (localhost):** https://localhost:28444
- **Web UI (NetBird):** https://<netbird-ip>:28444
- **API (localhost):** http://localhost:8080/api/
- **API (NetBird):** http://<netbird-ip>:8080/api/

### Android App Setup

1. Install [NetBird](https://play.google.com/store/apps/details?id=io.netbird.client) on your phone
2. Connect using management URL `https://your-netbird-server.example.com` and your setup key
3. In the Reminisce Android app, set the server to `<netbird-ip>:8080`

### Configuration Files

- `config-fullstack.yaml` — Reminisce server config (DBs, P2P peers, AI endpoints)
- `nginx-fullstack.conf` — Nginx reverse proxy config
- `generate-ssl-certs-fullstack.sh` — SSL cert generation with NetBird IP as SAN

### Useful Commands

```bash
# View server logs
docker logs -f reminisce-dev-server

# View nginx logs
docker logs -f reminisce-dev-nginx

# Check NetBird overlay IP
docker exec netbird-client netbird status | grep "NetBird IP"

# Regenerate SSL certs (e.g. after NetBird IP change)
docker compose -f docker-compose-dev.yml stop nginx
docker volume rm reminisce_nginx_dev_ssl
docker compose -f docker-compose-dev.yml up -d nginx

# Restart everything
docker compose -f docker-compose-dev.yml down
docker compose -f docker-compose-dev.yml up -d
```

---
## Running Tests

Tests use the dev databases (same as development). Each test creates a temporary database with a unique name, runs the test, and cleans up automatically.

```bash
# Using the test script (recommended - starts databases if needed)
./test.sh

# Or manually with cargo test (requires dev databases to be running)
cargo test

# Run specific test
cargo test auth_test

# Run with logs
RUST_LOG=debug cargo test -- --nocapture

# Just start databases for testing
./test.sh start

# Stop reminder (databases are shared with dev environment)
./test.sh stop
```

**How tests work:**
- Tests connect to the dev PostgreSQL server (port 5432)
- Each test gets a fresh, isolated database (e.g., `test_abc123def456`)
- Tests run in parallel without interfering with each other
- Temporary databases are automatically cleaned up after tests complete
- The main dev database (`reminisce_db`) is never affected by tests

---

## Debugging

### Backend (Rust)

**Using VS Code:**
1. Install "rust-analyzer" extension
2. Set breakpoints in `.rs` files
3. Press F5 to start debugging

**Using logs:**
```bash
# Verbose logging
RUST_LOG=debug cargo run --bin reminisce_bin -- config-dev.yml

# Module-specific logging
RUST_LOG=reminisce::services::auth=debug cargo run --bin reminisce_bin -- config-dev.yml
```

### Frontend (React)

**Browser DevTools:**
1. Open Chrome/Firefox DevTools (F12)
2. Use React DevTools extension
3. Check Network tab for API calls
4. Console for JavaScript errors

---

## Stopping Everything

```bash
# Stop all Docker services (databases, AI services, nginx)
docker compose -f docker-compose-dev.yml down

# Stop reminisce (in terminal: Ctrl+C)

# Stop client (in terminal: Ctrl+C)
```

---

## Common Issues & Solutions

### Issue: Images not found / Failed to pull
**Solution:** Build images first (one-time setup)
```bash
docker compose -f docker-compose-build.yml build
```

### Issue: Database connection refused
**Solution:** Ensure databases are running
```bash
docker compose -f docker-compose-dev.yml ps
```

### Issue: Port 8080 already in use
**Solution:** Find and kill the process
```bash
lsof -i :8080
kill -9 <PID>
```

### Issue: Port 5432 already in use (PostgreSQL)
**Solution:** Either stop local PostgreSQL or change port in docker-compose-dev.yml

### Issue: "uploaded_images" directory not found
**Solution:**
```bash
mkdir -p uploaded_images uploaded_videos
```

### Issue: Cargo build errors after adding dependencies
**Solution:**
```bash
cargo clean
cargo build
```

### Issue: AI service not starting or very slow
**Solution 1:** Check if model is downloading
```bash
docker logs -f reminisce-dev-ai
# Look for "Downloading model..." messages
```

**Solution 2:** Out of memory
- The 2B model requires ~8GB RAM
- Check available memory: `free -h`
- Consider closing other applications

**Solution 3:** Port 8081 already in use (AI service)
```bash
lsof -i :8081
kill -9 <PID>
```


**Solution 1:** Check if model files are present
```bash
ls -lh ai/models/
# Should see GGUF model files
```

**Solution 2:** Port 5000 already in use
```bash
lsof -i :5000
kill -9 <PID>
```


**Solution:**
- CPU inference is slow (30-120s per image)
- Increase timeout in Rust code if needed
- For faster inference, ensure GPU is available and detected
- Check logs: `docker logs reminisce-dev-ai`
- Check logs: `docker logs reminisce-dev-face`
- Verify model files exist in `ai/models/` directory

### Issue: Browser shows "Your connection is not private" or SSL error
**Solution:**
- This is normal for self-signed certificates in development
- Click "Advanced" → "Proceed to localhost (unsafe)"
- The connection is encrypted, just not verified by a CA

### Issue: Port 28080 or 28443 already in use
**Solution:**
```bash
lsof -i :28080
lsof -i :28443
kill -9 <PID>
```

### Issue: Nginx can't connect to backend or Vite
**Solution:**
- Ensure reminisce is running on port 8080
- Ensure Vite dev server is running on port 5173
- Check nginx logs: `docker logs reminisce-dev-nginx`
- Verify `host.docker.internal` is accessible from container

---

## Quick Start Script

**First time setup:**
```bash
# Build all images (do this once)
docker compose -f docker-compose-build.yml build
```

**Start development environment:**
```bash
./dev-start.sh
```

This script will:
- Start all Docker services (databases, AI service, P2P nodes, nginx)
- Create upload directories
- Display connection information and next steps

Then run in separate terminals:
```bash
# Terminal 1: Backend
RUST_LOG=info cargo run --bin reminisce_bin -- config-dev.yml

# Terminal 2: Frontend
cd client && npm run dev
```

---

## Production vs Development

| Aspect | Development | Production |
|--------|-------------|------------|
| Databases | Docker (localhost:5432, 5435) | Docker (internal network) |
| AI Service | Docker (localhost:8081) | Docker (internal network) |

| Nginx | Docker (localhost:28443 HTTPS) | Docker (internal network) |
| Backend | `cargo run` (localhost:8080) | Docker image |
| Frontend | `npm run dev` (localhost:5173) | Built & served by Nginx |
| HTTPS | Self-signed (dev) | Self-signed/Real certs |
| Tests | Use dev databases | Separate test databases |
| Hot Reload | ✅ Yes | ❌ No |
| Build Time | Instant | ~2-5 minutes |

---

## GPU Acceleration (Automatic)

The AI service automatically detects and use available GPUs (Intel, AMD, or NVIDIA) via `/dev/dri` device access. No special configuration needed!

### How It Works

Both development and production environments enable GPU support by default:
- **Intel GPUs**: Automatically detected via `/dev/dri`
- **AMD GPUs**: Automatically detected via `/dev/dri`
- **NVIDIA GPUs**: Automatically detected via `/dev/dri`
- **No GPU**: Automatically falls back to CPU

The AI services will automatically use the best available hardware at startup.

### Check GPU Status

**Verify GPU is detected:**
```bash
docker compose -f docker-compose-dev.yml logs clip-server | grep "detected\|loaded\|device"
```

You'll see one of:
- `NVIDIA GPU detected: <model>` - Using NVIDIA GPU
- `Intel GPU (XPU) detected` - Using Intel GPU
- `AMD GPU detected` - Using AMD GPU
- `No GPU detected - using CPU` - Using CPU

### Performance Comparison

Expected embedding generation speed (per image):

| Device | Speed | Notes |
|--------|-------|-------|
| NVIDIA RTX 4090 | ~50ms | Excellent |
| NVIDIA RTX 3070 | ~80ms | Very good |
| AMD RX 7900 XT | ~100ms | Good |
| Intel Arc A770 | ~150ms | Moderate |
| CPU (12-core) | ~500-1000ms | Baseline |

### Verify Performance

Test embedding speed:
```bash
time curl -X POST http://localhost:8081/embed/text \
  -H "Content-Type: application/json" \
  -d '{"text": "test"}'
```

- GPU: ~0.05-0.15 seconds
- CPU: ~0.5-1.0 seconds

### Troubleshooting GPU

**GPU not detected:**
```bash
# Check if /dev/dri exists
ls -la /dev/dri

# Check container can access GPU
docker exec reminisce-dev-ai ls -la /dev/dri
```

**Permission issues:**
```bash
# Add your user to video/render groups
sudo usermod -a -G video,render $USER
# Log out and back in
```

---

## Next Steps

1. ✅ Build images (first time): `docker compose -f docker-compose-build.yml build`
2. 🚀 Set up the development environment using `./dev-start.sh`
3. 🔧 Start backend: `RUST_LOG=info cargo run --bin reminisce_bin -- config-dev.yml`
4. 🎨 Start frontend: `cd client && npm run dev`
5. 📝 Create a test user via `/auth/register`
6. 🔐 Test login via `/auth/user-login`
7. 🖼️ Upload an image using the authenticated token
8. 🧪 Run tests with `./test.sh`
9. 🚀 When ready, rebuild Docker images: `docker compose -f docker-compose-build.yml build`

Happy coding! 🎉
