# Deployment

## Prerequisites

- Docker + Docker Compose (for containerised deployment)
- PostgreSQL 15+ with `pgvector` and `postgis` extensions
- TLS certificate (self-signed is fine; `actix-web` with `rustls`)
- At least one storage node running the `np2p` daemon (Raspberry Pi or any Linux machine)

## Docker Compose Files

| File | Use |
|------|-----|
| `docker-compose.yml` | Production: API, AI service, Postgres, nginx |
| `docker/docker-compose-dev.yml` | Development with hot-reload and exposed debug ports |
| `docker/docker-compose-build.yml` | Build images locally (used before pushing to registry) |
| `docker/docker-compose-observability.yml` | Prometheus + Loki + Grafana monitoring stack |

Start everything:
```bash
docker compose up -d
docker compose -f docker/docker-compose-observability.yml up -d  # optional
```

## First-Run Setup

On a fresh database there are no users — `generate_install.sh` generates a random
`api_secret_key` and does **not** pre-seed an admin account. Open the web UI and the
first visitor creates the admin via the setup screen.

If you must script it, the API still accepts an explicit first-admin creation:

```bash
curl -X POST http://localhost:8080/api/auth/setup \
  -H 'Content-Type: application/json' \
  -d '{"username": "admin", "password": "<random>"}'
```

This endpoint returns `403` once any user exists, so it's safe to call idempotently.

## Configuration (`config.yaml`)

Copy `config-fullstack.yaml.example` to `config-fullstack.yaml`. Required fields:

### Secret management (`api_secret_key`)

`api_secret_key` is the **master secret** for the whole system — JWT signing, backup-key wrapping, the P2P node identity (when `p2p_deterministic_identity` is on), and mesh-manifest encryption all chain off it. Treat it accordingly:

- `generate_install.sh` generates a **random 64-hex value** for `config.yaml` automatically; you don't need to invent one.
- **Prefer env injection** over the file: set `API_SECRET_KEY` in the environment (it takes precedence over `config.yaml`, so the secret never touches disk).
- If it must live in `config.yaml`, lock the file down: `chmod 600 config.yaml`. The server **warns at startup** if the file is readable by group/others.
- Rotate it with care: rotating invalidates existing JWTs and changes the deterministic P2P node identity (existing shard tokens/owner-pins must be re-established).

| Key | Required | Description |
|-----|----------|-------------|
| `api_secret_key` | Yes | JWT signing key — `openssl rand -base64 32` (or set `API_SECRET_KEY` env var) |
| `database_url` | Yes | `postgres://user:pass@host:5432/reminisce` |
| `geotagging_database_url` | Yes | Separate PostGIS DB with offline geo data |
| `images_dir` | Yes | Absolute path where uploaded images are stored |
| `videos_dir` | Yes | Absolute path where uploaded videos are stored |
| `ai_grpc_url` | Yes | AI service gRPC URL for all inference (default `http://localhost:50051`; use `http://ai-server:50051` when the backend runs inside the compose network). Legacy `embedding_service_url`/`face_service_url` were removed. |
| `p2p_data_dir` | Yes | Directory for P2P node identity and shard storage |
| `p2p_namespace` | Yes | Namespace to isolate peer groups (e.g. `production`, `home`) |
| `port` | No | HTTP listen port (default `8080`) |
| `p2p_coordinator_addr` | No | `host:port` of coordinator for WAN peer discovery |
| `p2p_coordinator_node_id` | Required with `p2p_coordinator_addr` | The coordinator's 64-hex Node ID (printed in the coordinator startup log). Bound to the QUIC connection so a spoofed coordinator cannot impersonate the real one. If set without `p2p_coordinator_addr` it is ignored; if `p2p_coordinator_addr` is set without it, coordinator/tunnel use is disabled with a clear error. |
| `p2p_discovery_port` | No | UDP port for LAN peer discovery broadcasts (default `5066`) |
| `p2p_deterministic_identity` | No | Derive P2P node identity from `api_secret_key` (default `false`). Enables true full-disk-loss recovery but changes node_id — use for new setups |
| `p2p_tunnel_local_port` | No | Local HTTP port the reverse tunnel should forward to |
| `otlp_endpoint` | No | OpenTelemetry OTLP gRPC endpoint for distributed tracing |
| `environment` | No | Label for tracing spans (`production`, `dev`) |

### Database backup worker (`workers:` section)

The periodic P2P database snapshot worker is configured under the `workers:` key:

| Key | Default | Description |
|-----|---------|-------------|
| `workers.db_backup_enabled` | `true` | Enable/disable the periodic DB snapshot worker |
| `workers.db_backup_interval_secs` | `86400` | Seconds between snapshots (default 24 h) |
| `workers.db_backup_retention_count` | `7` | Number of snapshots to keep in the rolling P2P manifest; older ones are pruned |

Requires `pg_dump` available on the server `PATH` (present in the Docker image).

Each snapshot also writes a self-contained restore manifest to `<p2p_data_dir>/db_manifests/<backup_hash>.json` (mode `0600`) so the database can be rebuilt even after a full DB loss. Keep `p2p_data_dir` on persistent storage and back it up alongside your media — it holds `node.key` (P2P identity) and the snapshot manifests needed for disaster recovery. To restore, use the `disaster_recovery` binary (see `docs/p2p-backup.md`).

## TLS / HTTPS

The server uses `rustls`. Put TLS termination in nginx (see `nginx/` at the repo root) rather than configuring rustls directly. The included nginx config handles:
- HTTPS on port 28444 (self-signed cert for LAN use)
- WebSocket proxying for live updates
- Static file serving for the React client
- On the HTTP listener, credential-bearing paths (`user-login-form`, `user-login`,
  `setup`) are **307-redirected to HTTPS** so passwords never traverse cleartext.

Session cookies are `Secure`, so an HTTP-only login can never establish a session —
**use HTTPS (or a VPN like NetBird) to protect the media traffic itself.**

## Storage Nodes (Pi Setup)

Each Pi runs the `np2p` daemon binary (`np2pd`):

```bash
cargo build --release --bin np2pd
scp target/release/np2pd pi@192.168.1.x:/usr/local/bin/

# On the Pi — pin the home server as the ONLY allowed owner (recommended):
np2pd --data-dir /mnt/disk/p2p \
  --coordinator-addr yourcoordinator.example.com:5055 \
  --coordinator-node-id <64-hex-coordinator-node-id> \
  --authorized-node-id <64-hex-home-server-node-id>
```

> **Security: storage nodes FAIL CLOSED.** `np2pd` refuses to start without
> `--authorized-node-id` **or** an explicit `--allow-unpinned` opt-in. Prefer pinning:
> an unpinned node accepts self-signed store/retrieve/delete tokens from any node. To
> find the home server's node id, check `${p2p_data_dir}/node_id.txt` on the server (or
> its startup log).

The home server will auto-discover Pi nodes via LAN UDP broadcast (`p2p_discovery_port`) or via the coordinator for WAN. `--coordinator-node-id` is mandatory whenever `--coordinator-addr` is used (identity binding); the value is printed in the coordinator's startup log. On the home server, set `p2p_coordinator_node_id` in the config (see the table above).

> **Coordinator tunnel**: on the VPS, register the home server as the tunnel backend with `--allowed-tunnel-node-id <home-server-node-id>` (refused by default). Never rely on the old "any node may register" default; if you must, pass `--allow-any-tunnel` explicitly.

## Observability

All components export metrics and traces:

| Component | Endpoint | Format |
|-----------|----------|--------|
| Reminisce API | `GET /metrics` | Prometheus text |
| AI service | `GET /metrics` | Prometheus text |
| Traces | → `otlp_endpoint` | OTLP gRPC (bring your own backend, e.g. Tempo/Jaeger) |

Grafana dashboards are in `observability/grafana/dashboards/`. Promtail scrapes Docker container logs into Loki.

> **Access control:** Grafana requires login (anonymous admin is disabled) and is bound
> to `127.0.0.1` together with Prometheus and Alloy. Loki log retention is **7 days**.
> Publish ports yourself only if you install your own auth in front.

Key metrics: `user_registrations_total`, `user_logins_total`, `user_login_failures_total`, `api_http_requests_total`, `api_http_request_duration_seconds`.

## Build and Push

```bash
# Build ARM64 image for Pi nodes
docker buildx build --platform linux/arm64 -t yourregistry/np2p-daemon:latest np2p/

# Build AMD64 image for home server
docker buildx build --platform linux/amd64 -t yourregistry/reminisce:latest .
```

> **Note:** Never cross-build the `p2p-node` image from an x86 host — the QUIC crypto benchmarks differ enough to cause subtle failures. Build on the Pi directly or use a native ARM64 runner.
