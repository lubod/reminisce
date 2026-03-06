# Reminisce

**Self-hosted personal media backup and gallery with AI-powered search.**

Reminisce lets you back up photos and videos from your phone to your own server, then find them by searching natural language descriptions, faces, locations, and dates — no cloud subscription required.

## Features

- **AI-powered search** — semantic similarity search using SigLIP2 embeddings; find photos by typing "beach sunset" or "birthday cake"
- **Automatic descriptions** — SmolVLM-500M generates text descriptions for every photo (~5s per image)
- **Face recognition** — InsightFace clusters faces into people; name them and search by person
- **Location tagging** — GPS reverse-geocoding (offline PostGIS database, no API key required)
- **P2P distributed storage** — files are erasure-coded and distributed across your own storage nodes over a WireGuard mesh (NetBird), with no single point of failure
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
         connected via NetBird WireGuard mesh
```

## Quick Start (Docker)

**Requirements:** Docker, Docker Compose, a NetBird account (free self-hosted)

```bash
# 1. Clone the repo
git clone https://github.com/lubod/reminisce.git
cd reminisce

# 2. Configure
cp config-fullstack.yaml.example config-fullstack.yaml
# Edit config-fullstack.yaml:
#   - Set api_secret_key (generate: openssl rand -base64 32)
#   - Set p2p_peers to your NetBird overlay IPs

cp .env.example .env
# Edit .env:
#   - Set NETBIRD_SETUP_KEY from your NetBird management panel
#   - Set NETBIRD_MANAGEMENT_URL to your NetBird server

# 3. Pull and start
docker compose -f docker-compose-dev.yml up -d
```

The web UI will be available at `https://localhost:28444` (self-signed cert).

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

## Configuration

Copy `config-fullstack.yaml.example` to `config-fullstack.yaml` and set:

| Key | Description |
|-----|-------------|
| `api_secret_key` | JWT signing key — `openssl rand -base64 32` |
| `p2p_peers` | List of `ip:port` for your storage nodes |
| `database_url` | PostgreSQL connection string |
| `embedding_service_url` | URL of the AI service (default `http://localhost:8081`) |

## 🔒 Security & Best Practices

To protect your personal media, follow these essential security steps after installation:

1.  **Generate a Unique Secret Key:** The `api_secret_key` is used to sign authentication tokens. Generate a random 32-character key and set it in your `config-fullstack.yaml`:
    ```bash
    openssl rand -base64 32
    ```
2.  **Change the Default Admin Password:** The default credentials are `admin` / `admin123`. **Log in immediately** and change your password in the User settings.
3.  **Use a Private Mesh:** Run Reminisce behind a private mesh VPN like **NetBird** (default) or **Tailscale**. Avoid exposing the API port (8080) directly to the public internet.
4.  **Hardware Encryption:** If your storage nodes are physically accessible to others, ensure their disks are encrypted (LUKS/FileVault).

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

## P2P Storage

Files are split into shards using Reed-Solomon erasure coding (5-of-8 by default) and distributed to storage nodes over a WireGuard mesh network (NetBird). Each shard is encrypted with ChaCha20Poly1305. Any 5 of 8 nodes can reconstruct any file.

See [np2p/DESIGN.md](np2p/DESIGN.md) for the full design document.

## NetBird Setup

Reminisce uses [NetBird](https://netbird.io) for secure overlay networking between nodes. You can use the hosted NetBird service or run your own server:

```bash
# Deploy a self-hosted NetBird management server
cd netbird
export NETBIRD_MANAGEMENT_URL=https://your-netbird-server.example.com
./manage.sh setup-server

# Create a setup key for a new node
./manage.sh create-key my-storage-node
```

## License

GNU Affero General Public License v3.0 — see [LICENSE](LICENSE).

The AGPL requires that if you run a modified version as a network service, you must make your source code available to users. This is intentional: modifications to a self-hosted media server should remain open.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Issues and pull requests welcome.
