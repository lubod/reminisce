# NetBird Mesh VPN for Reminisce P2P

Reminisce uses **NetBird** for secure, encrypted P2P connectivity between storage nodes. NetBird creates a WireGuard mesh network with automatic NAT traversal and relay fallback.

## Architecture

```
  your-netbird-server.example.com (NetBird Server)
  ┌─────────────────────────────────┐
  │ Management + Signal + Relay     │
  │ + Dashboard + Traefik (TLS)     │
  │ Ports: 80, 443, 3478/UDP       │
  └─────────────────────────────────┘
           │
     WireGuard mesh (automatic)
           │
  ┌────────┼──────────┬─────────────┐
  │        │          │             │
Host A   Host B    Host C      Android
(server  (storage  (storage    (NetBird
+storage) node)    node)       mobile app)
```

Each Docker host runs one NetBird client container (`network_mode: host`). All app containers on that host share the WireGuard overlay IP.

## Quick Start

### 1. Deploy NetBird Server

On `your-netbird-server.example.com`:

```bash
./manage.sh setup-server
```

This runs the official NetBird quickstart, deploying the Management API, Signal server, Relay, STUN, and Traefik with auto-TLS.

### 2. Create a Personal Access Token

1. Open `https://your-netbird-server.example.com`
2. Go to Settings > Personal Access Tokens
3. Create a token and add it to `.env`:

```bash
cp .env.example .env
# Edit .env and set NETBIRD_PAT=<your-token>
```

### 3. Create Setup Keys

```bash
./manage.sh create-key dev-nodes
```

This creates a reusable setup key (30-day expiry) and saves it to `keys/dev-nodes.key`.

### 4. Deploy Nodes

Set the setup key in your environment:

```bash
# In the directory with docker-compose-netbird.yml
echo "NETBIRD_SETUP_KEY=$(cat netbird/keys/dev-nodes.key)" >> .env
```

Start the stack:

```bash
docker compose -f docker-compose-netbird.yml up -d
```

Verify connectivity:

```bash
docker exec netbird-client netbird status
# or
./manage.sh status
```

### 5. Configure Reminisce

Once nodes are connected, find their NetBird IPs in the dashboard and update `config.yaml`:

```yaml
p2p_peers:
  - "100.x.x.1:5050"  # storage node 1
  - "100.x.x.2:5050"  # storage node 2
  - "100.x.x.3:5050"  # storage node 3
```

### 6. Add Mobile Devices

```bash
./manage.sh add-mobile my-phone
```

Follow the printed instructions to install the NetBird Android app and connect with the setup key.

## Management Commands

| Command | Description |
|---|---|
| `./manage.sh setup-server` | Deploy NetBird server (run on your-netbird-server.example.com) |
| `./manage.sh create-key <name>` | Create a reusable setup key |
| `./manage.sh add-node <name>` | Print docker-compose snippet for a remote storage node |
| `./manage.sh add-mobile <name>` | Print mobile enrollment instructions + setup key |
| `./manage.sh status` | Show connected peers |
| `./manage.sh list` | List all peers via Management API |

## Why NetBird?

- **Relay fallback**: If hole-punching fails, traffic routes through relay automatically
- **No manual certs**: Setup keys instead of PKI certificate management
- **Built-in dashboard**: Web UI at `https://your-netbird-server.example.com`
- **WireGuard**: Battle-tested encryption, not custom crypto
- **Easy mobile**: Android app + setup key enrollment
