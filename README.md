# Reminisce

**Self-hosted personal media backup and gallery with AI-powered search.**

Reminisce lets you back up photos and videos from your phone to your own server, then find them by searching natural language descriptions, faces, locations, and dates — no cloud subscription required.

## Features

- **AI-powered search** — semantic similarity search using SigLIP2 embeddings; find photos by typing "beach sunset" or "birthday cake"
- **Automatic descriptions** — SmolVLM-500M generates text descriptions for every photo (~5s per image)
- **Face recognition** — InsightFace clusters faces into people; name them and search by person
- **Location tagging** — GPS reverse-geocoding (offline PostGIS database, no API key required)
- **P2P distributed storage** — files are erasure-coded (3/5 Reed-Solomon) and distributed across your own storage nodes, with no single point of failure
- **Cross-device deduplication** — the same photo from multiple phones is stored once
- **Android client** — automatic background backup from your phone
- **AGPL v3** — fully open source, self-hostable, no telemetry

## Architecture

```
Android / Web client
        │
        ▼
 Reminisce API (Rust/Actix-web)
   ├── PostgreSQL + pgvector + PostGIS
   ├── AI service (Python/Flask)
   │     ├── SigLIP2 — image & text embeddings
   │     ├── SmolVLM-500M — image descriptions
   │     └── InsightFace — face detection (CPU)
   └── np2p storage nodes (QUIC/ChaCha20Poly1305)
```

## Documentation

| Document | Description |
|----------|-------------|
| [docs/architecture.md](docs/architecture.md) | System diagram, crate responsibilities, workers, auth |
| [docs/p2p-backup.md](docs/p2p-backup.md) | P2P erasure coding, encryption, repair, restore |
| [docs/database.md](docs/database.md) | Table reference, index strategy, migrations |
| [docs/deployment.md](docs/deployment.md) | Docker setup, config reference, storage nodes, observability |
| [DEV_SETUP.md](DEV_SETUP.md) | Local development guide |
| Swagger UI | `http://localhost:8080/swagger-ui/` (live API explorer when running) |

## Quick Start (Docker)

**Requirements:** Docker, Docker Compose

```bash
# 1. Clone the repo
git clone https://github.com/lubod/reminisce.git
cd reminisce

# 2. Configure
cp config-fullstack.yaml.example config-fullstack.yaml
# Edit config-fullstack.yaml:
#   - Set api_secret_key (generate: openssl rand -base64 32)
#   - Set p2p_coordinator_addr to your coordinator's address

# 3. Pull and start
docker compose up -d
```

The web UI will be available at `https://localhost:28444` (self-signed cert).

On first run, create the admin account:
```bash
curl -X POST http://localhost:8080/api/auth/setup \
  -H 'Content-Type: application/json' \
  -d '{"username": "admin", "password": "your-secure-password"}'
```

## Development Setup

See [DEV_SETUP.md](DEV_SETUP.md) for a full local development guide with hot-reload for both the Rust backend and React frontend.

```bash
# Start infrastructure (databases, AI, P2P nodes, nginx)
./dev start

# Run the Rust server with live reload
./dev watch

# Run the React client (hot module replacement)
./dev client

# Run all tests
./dev test
```

## Security

- **Generate a unique `api_secret_key`:** `openssl rand -base64 32`
- **Don't expose port 8080** to the internet — run behind a VPN or firewall
- **Encrypt storage node disks** (LUKS/FileVault) if physically accessible to others

## AI Service

The AI service runs as a Docker container (`lubod/reminisce-ai-server`) and exposes:

| Endpoint | Model | Speed |
|----------|-------|-------|
| `POST /embed/image` | SigLIP2 (1152-dim) | ~430ms |
| `POST /embed/text` | SigLIP2 (1152-dim) | ~25ms |
| `POST /describe` | SmolVLM-500M-Instruct | ~5.5s |
| `POST /describe/qwen` | Qwen2.5-VL-3B | ~28s |
| `POST /detect` | InsightFace buffalo_l | ~625ms |

GPU acceleration is supported for AMD ROCm and CUDA. InsightFace runs on CPU to avoid ROCm ONNX crashes on RDNA3 iGPUs.

## License

GNU Affero General Public License v3.0 — see [LICENSE](LICENSE).

The AGPL requires that if you run a modified version as a network service, you must make your source code available to users. This is intentional: modifications to a self-hosted media server should remain open.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Issues and pull requests welcome.
