#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENV_FILE="$SCRIPT_DIR/.env"
KEYS_DIR="$SCRIPT_DIR/keys"
# Set NETBIRD_MANAGEMENT_URL in netbird/.env, or export it before running this script
MANAGEMENT_URL="${NETBIRD_MANAGEMENT_URL:-https://your-netbird-server.example.com}"
API_BASE="$MANAGEMENT_URL/api"

# Load environment variables if .env exists
if [ -f "$ENV_FILE" ]; then
    # shellcheck disable=SC1090
    source "$ENV_FILE"
fi

usage() {
    cat <<EOF
Usage: $(basename "$0") <command> [args]

NetBird mesh VPN management for Reminisce P2P

Commands:
  setup-server          Run the NetBird quickstart on the current machine
  create-key <name>     Create a reusable setup key via Management API
  add-node <name>       Generate a docker-compose snippet for a storage node
  add-mobile <name>     Print setup key + instructions for Android NetBird app
  status                Show connected peers (runs netbird status)
  list                  List all peers via Management API

Environment:
  Set NETBIRD_PAT in $ENV_FILE for API access.
  See .env.example for template.
EOF
    exit 1
}

require_pat() {
    if [ -z "${NETBIRD_PAT:-}" ]; then
        echo "Error: NETBIRD_PAT not set. Add it to $ENV_FILE"
        echo "  You can create a PAT in the NetBird dashboard: $MANAGEMENT_URL"
        exit 1
    fi
}

api_call() {
    local method="$1"
    local endpoint="$2"
    local data="${3:-}"

    local args=(
        -s -f
        -H "Authorization: Token $NETBIRD_PAT"
        -H "Content-Type: application/json"
        -X "$method"
    )
    if [ -n "$data" ]; then
        args+=(-d "$data")
    fi

    curl "${args[@]}" "$API_BASE$endpoint"
}

cmd_setup_server() {
    echo "Setting up NetBird server on this machine..."
    echo "Domain: ${NETBIRD_MANAGEMENT_URL:-your-netbird-server.example.com}"
    echo ""
    echo "Prerequisites:"
    echo "  - Ports 80, 443 (TCP) and 3478 (UDP) must be available"
    echo "  - DNS A record pointing to this machine"
    echo ""
    read -rp "Continue? [y/N] " confirm
    if [[ "$confirm" != [yY] ]]; then
        echo "Aborted."
        exit 0
    fi

    echo "Downloading and running NetBird quickstart..."
    curl -fsSL https://github.com/netbirdio/netbird/releases/latest/download/getting-started-with-zitadel.sh -o /tmp/netbird-quickstart.sh
    chmod +x /tmp/netbird-quickstart.sh
    bash /tmp/netbird-quickstart.sh
    rm -f /tmp/netbird-quickstart.sh

    echo ""
    echo "NetBird server setup complete!"
    echo "Dashboard: $MANAGEMENT_URL"
    echo ""
    echo "Next steps:"
    echo "  1. Log into the dashboard and create a Personal Access Token (PAT)"
    echo "  2. Add NETBIRD_PAT=<your-token> to $ENV_FILE"
    echo "  3. Run: $(basename "$0") create-key <name>"
}

cmd_create_key() {
    local name="${1:-}"
    if [ -z "$name" ]; then
        echo "Usage: $(basename "$0") create-key <name>"
        echo "Example: $(basename "$0") create-key dev-nodes"
        exit 1
    fi

    require_pat
    mkdir -p "$KEYS_DIR"

    echo "Creating setup key '$name'..."

    # Create a reusable setup key, valid for 30 days
    local expires_in=2592000  # 30 days in seconds
    local response
    response=$(api_call POST "/setup-keys" "{
        \"name\": \"$name\",
        \"type\": \"reusable\",
        \"expires_in\": $expires_in,
        \"auto_groups\": [\"All\"],
        \"usage_limit\": 0,
        \"ephemeral\": false
    }")

    local key
    key=$(echo "$response" | python3 -c "import sys,json; print(json.load(sys.stdin)['key'])" 2>/dev/null || true)

    if [ -z "$key" ]; then
        echo "Error creating setup key. Response:"
        echo "$response" | python3 -m json.tool 2>/dev/null || echo "$response"
        exit 1
    fi

    echo "$key" > "$KEYS_DIR/$name.key"
    echo ""
    echo "Setup key created: $key"
    echo "Saved to: $KEYS_DIR/$name.key"
    echo ""
    echo "Use this key in .env:"
    echo "  NETBIRD_SETUP_KEY=$key"
}

cmd_add_node() {
    local name="${1:-}"
    if [ -z "$name" ]; then
        echo "Usage: $(basename "$0") add-node <name>"
        echo "Example: $(basename "$0") add-node storage-host-1"
        exit 1
    fi

    cat <<EOF
# Docker Compose snippet for storage node: $name
# Add this to your docker-compose-netbird.yml or use as a standalone file.
# Set NETBIRD_SETUP_KEY in your .env file.

services:
  netbird:
    image: netbirdio/netbird:latest
    container_name: netbird-client
    network_mode: host
    cap_add: [NET_ADMIN, SYS_ADMIN, SYS_RESOURCE]
    devices: ["/dev/net/tun:/dev/net/tun"]
    volumes:
      - netbird_config:/etc/netbird
    environment:
      NB_SETUP_KEY: "\${NETBIRD_SETUP_KEY}"
      NB_MANAGEMENT_URL: "$MANAGEMENT_URL"
    restart: unless-stopped
    healthcheck:
      test: ["CMD", "netbird", "status"]
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 15s

  storage-node:
    image: lubod/reminisce-p2p:latest
    container_name: p2p-$name
    network_mode: host
    command: np2pd --listen 0.0.0.0:5050 --data-dir /data
    depends_on:
      netbird: { condition: service_healthy }
    volumes:
      - p2p_data:/data
    restart: unless-stopped

volumes:
  netbird_config:
  p2p_data:
EOF
}

cmd_add_mobile() {
    local name="${1:-mobile}"

    require_pat

    # Check if a key already exists for this name
    local key_file="$KEYS_DIR/$name.key"
    local key=""

    if [ -f "$key_file" ]; then
        key=$(cat "$key_file")
        echo "Using existing setup key for '$name'."
    else
        echo "Creating setup key for mobile device '$name'..."
        cmd_create_key "$name" > /dev/null 2>&1
        if [ -f "$key_file" ]; then
            key=$(cat "$key_file")
        fi
    fi

    if [ -z "$key" ]; then
        echo "Error: Could not create or retrieve setup key."
        exit 1
    fi

    # Look up the Reminisce server's NetBird IP from the management API
    local server_netbird_ip=""
    local peers_response
    peers_response=$(api_call GET "/peers" 2>/dev/null || true)

    if [ -n "$peers_response" ]; then
        server_netbird_ip=$(echo "$peers_response" | python3 -c "
import sys, json
try:
    peers = json.load(sys.stdin)
    for p in peers:
        pname = p.get('name', '').lower()
        if 'server' in pname or 'reminisce' in pname:
            print(p.get('ip', ''))
            break
except Exception:
    pass
" 2>/dev/null || true)
    fi

    cat <<EOF

Mobile Enrollment Instructions
==============================

Step 1 — Install NetBird on your Android device
  https://play.google.com/store/apps/details?id=io.netbird.client

Step 2 — Connect to the Reminisce mesh
  Open NetBird → tap "Add with setup key"
  Management URL: $MANAGEMENT_URL
  Setup Key:      $key
  Tap Connect. Your phone will receive a 100.x.x.x WireGuard IP.

Step 3 — Configure the Reminisce Android app
EOF

    if [ -n "$server_netbird_ip" ]; then
        local mgmt_host
        mgmt_host=$(echo "${NETBIRD_MANAGEMENT_URL:-}" | sed 's|https\?://||')
        local reminisce_json="{\"server_url\": \"http://$server_netbird_ip:8080\", \"node_id\": \"\", \"rendezvous_url\": \"${mgmt_host:-your-netbird-server.example.com}:5051\"}"
        cat <<EOF
  Server NetBird IP detected: $server_netbird_ip
  Server URL:  http://$server_netbird_ip:8080

  Tap "Paste JSON" in the Reminisce app and enter:
  $reminisce_json
EOF
        if command -v qrencode &>/dev/null; then
            echo ""
            echo "  Scan this QR code in the Reminisce app:"
            qrencode -t ANSIUTF8 "$reminisce_json"
        fi
    else
        cat <<EOF
  Could not auto-detect the server's NetBird IP.
  Check the dashboard at $MANAGEMENT_URL or run: $(basename "$0") list
  Then in the Reminisce app set Server URL to: http://<server-ip>:8080
EOF
    fi

    echo ""
    echo "All traffic routes over the encrypted WireGuard mesh. No port forwarding needed."
}

cmd_status() {
    if command -v netbird &> /dev/null; then
        netbird status
    elif docker ps --format '{{.Names}}' | grep -q netbird-client; then
        docker exec netbird-client netbird status
    else
        echo "NetBird client not found. Is it running?"
        echo "  Try: docker exec netbird-client netbird status"
        exit 1
    fi
}

cmd_list() {
    require_pat
    echo "Fetching peers from $API_BASE/peers..."
    echo ""

    local response
    response=$(api_call GET "/peers")

    echo "$response" | python3 -c "
import sys, json

peers = json.load(sys.stdin)
if not peers:
    print('No peers found.')
    sys.exit(0)

fmt = '{:<20} {:<16} {:<12} {:<20}'
print(fmt.format('NAME', 'IP', 'STATUS', 'LAST SEEN'))
print('-' * 70)
for p in peers:
    name = p.get('name', 'unknown')
    ip = p.get('ip', 'N/A')
    connected = 'connected' if p.get('connected', False) else 'offline'
    last_seen = p.get('last_seen', 'N/A')[:19] if p.get('last_seen') else 'N/A'
    print(fmt.format(name, ip, connected, last_seen))
" 2>/dev/null || {
        echo "Raw response:"
        echo "$response" | python3 -m json.tool 2>/dev/null || echo "$response"
    }
}

# --- Main ---
if [ $# -eq 0 ]; then
    usage
fi

case "$1" in
    setup-server) cmd_setup_server ;;
    create-key)   cmd_create_key "${2:-}" ;;
    add-node)     cmd_add_node "${2:-}" ;;
    add-mobile)   cmd_add_mobile "${2:-}" ;;
    status)       cmd_status ;;
    list)         cmd_list ;;
    *)            usage ;;
esac
