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
| `docker/docker-compose-observability.yml` | Prometheus + Loki + Grafana + Tempo monitoring stack |

Start everything:
```bash
docker compose up -d
docker compose -f docker/docker-compose-observability.yml up -d  # optional
```

## First-Run Setup

On a fresh database there are no users. Create the first admin account:

```bash
curl -X POST http://localhost:8080/api/auth/setup \
  -H 'Content-Type: application/json' \
  -d '{"username": "admin", "password": "changeme123"}'
```

This endpoint returns `403` if any user already exists, so it's safe to call idempotently.

## Configuration (`config.yaml`)

Copy `config-fullstack.yaml.example` to `config-fullstack.yaml`. Required fields:

| Key | Required | Description |
|-----|----------|-------------|
| `api_secret_key` | Yes | JWT signing key — `openssl rand -base64 32` |
| `database_url` | Yes | `postgres://user:pass@host:5432/reminisce` |
| `geotagging_database_url` | Yes | Separate PostGIS DB with offline geo data |
| `images_dir` | Yes | Absolute path where uploaded images are stored |
| `videos_dir` | Yes | Absolute path where uploaded videos are stored |
| `embedding_service_url` | Yes | AI service base URL (default `http://localhost:8081`) |
| `face_service_url` | Yes | AI face detection URL (often same as embedding service) |
| `p2p_data_dir` | Yes | Directory for P2P node identity and shard storage |
| `p2p_namespace` | Yes | Namespace to isolate peer groups (e.g. `production`, `home`) |
| `port` | No | HTTP listen port (default `8080`) |
| `p2p_coordinator_addr` | No | `host:port` of coordinator for WAN peer discovery |
| `p2p_discovery_port` | No | UDP port for LAN peer discovery broadcasts (default `5056`) |
| `p2p_tunnel_local_port` | No | Local HTTP port the reverse tunnel should forward to |
| `otlp_endpoint` | No | OpenTelemetry OTLP gRPC endpoint for distributed tracing |
| `environment` | No | Label for tracing spans (`production`, `dev`) |

## TLS / HTTPS

The server uses `rustls`. Put TLS termination in nginx (see `docker/nginx/`) rather than configuring rustls directly. The included nginx config handles:
- HTTPS on port 28444 (self-signed cert for LAN use)
- WebSocket proxying for live updates
- Static file serving for the React client

## Storage Nodes (Pi Setup)

Each Pi runs the `np2p` daemon binary:

```bash
cargo build --release --bin np2p_daemon
scp target/release/np2p_daemon pi@192.168.1.x:/usr/local/bin/

# On the Pi:
np2p_daemon --data-dir /mnt/disk/p2p --coordinator yourcoordinator.example.com:5055
```

The home server will auto-discover Pi nodes via LAN UDP broadcast (`p2p_discovery_port`) or via the coordinator for WAN.

## Observability

All components export metrics and traces:

| Component | Endpoint | Format |
|-----------|----------|--------|
| Reminisce API | `GET /metrics` | Prometheus text |
| AI service | `GET /metrics` | Prometheus text |
| Traces | → `otlp_endpoint` | OTLP gRPC → Tempo |

Grafana dashboards are in `observability/grafana/dashboards/`. Promtail scrapes Docker container logs into Loki.

Key metrics: `user_registrations_total`, `user_logins_total`, `user_login_failures_total`, `api_http_requests_total`, `api_http_request_duration_seconds`.

## Build and Push

```bash
# Build ARM64 image for Pi nodes
docker buildx build --platform linux/arm64 -t yourregistry/np2p-daemon:latest np2p/

# Build AMD64 image for home server
docker buildx build --platform linux/amd64 -t yourregistry/reminisce:latest .
```

> **Note:** Never cross-build the `p2p-node` image from an x86 host — the QUIC crypto benchmarks differ enough to cause subtle failures. Build on the Pi directly or use a native ARM64 runner.
