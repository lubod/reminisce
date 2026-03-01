#!/bin/bash
set -e

# =============================================================================
# E2E Test: Streaming Relay Proxy
#
# Verifies the multiplexed binary streaming proxy:
#   1. Relay starts
#   2. Home Server connects to Relay via WebSocket
#   3. Client uploads a "large" file to Relay proxy -> Home Server receives stream
#   4. Client downloads the file from Relay proxy -> Home Server sends stream
# =============================================================================

RELAY_PORT=18188
HOME_PORT=18189
PG_PORT=15434
PG_CONTAINER="e2e-streaming-postgres-$$"
TEST_DIR="test_streaming_env"
RELAY_BINARY="./target/debug/relay"
HOME_BINARY="./target/debug/reminisce"

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
NC='\033[0m'

PASS=0
FAIL=0

function cleanup {
    echo ""
    echo -e "${GREEN}Cleaning up...${NC}"
    pkill -P $$ 2>/dev/null || true
    pkill -f "$RELAY_BINARY" 2>/dev/null || true
    pkill -f "$HOME_BINARY" 2>/dev/null || true
    sleep 1
    docker stop "$PG_CONTAINER" 2>/dev/null || true
    docker rm "$PG_CONTAINER" 2>/dev/null || true
    [ -f "large_test_payload.bin" ] && rm -f "large_test_payload.bin"
    [ -f "downloaded_payload.bin" ] && rm -f "downloaded_payload.bin"
    echo -e "${GREEN}Done.${NC}"
}
trap cleanup EXIT

function check_pass {
    local desc="$1"
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}PASS${NC}: $desc"
}

function check_fail {
    local desc="$1"
    FAIL=$((FAIL + 1))
    echo -e "  ${RED}FAIL${NC}: $desc"
}

# ============================================================
# BUILD
# ============================================================
echo -e "${CYAN}=== Building binaries ===${NC}"
cargo build --bin relay -p relay 2>&1 | tail -1
cargo build --bin reminisce 2>&1 | tail -1

# ============================================================
# START POSTGRES
# ============================================================
echo -e "${CYAN}=== Starting PostgreSQL ===${NC}"
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR"

docker build -t reminisce-postgres -f Dockerfile.postgres . > /dev/null

docker run -d --name "$PG_CONTAINER" \
    -e POSTGRES_USER=reminisce -e POSTGRES_PASSWORD=reminisce -e POSTGRES_DB=reminisce \
    -p "$PG_PORT:5432" \
    reminisce-postgres >/dev/null

for i in {1..30}; do
    if docker exec "$PG_CONTAINER" pg_isready -U reminisce -q 2>/dev/null; then break; fi
    sleep 1
done
echo -e "${GREEN}PostgreSQL ready on port $PG_PORT${NC}"

echo "Initializing database schema..."
docker exec -i "$PG_CONTAINER" psql -U reminisce -d reminisce < init.sql > /dev/null
echo -e "${GREEN}Database schema initialized.${NC}"

# ============================================================
# START RELAY
# ============================================================
echo -e "${CYAN}=== Starting Relay ===${NC}"
DATABASE_URL="postgres://reminisce:reminisce@127.0.0.1:$PG_PORT/reminisce" \
    JWT_SECRET="relay-test-secret" \
    PORT="$RELAY_PORT" \
    SHARD_STORAGE_DIR="$TEST_DIR/shards" \
    RUST_LOG="info" \
    $RELAY_BINARY > "$TEST_DIR/relay.log" 2>&1 &

RELAY_URL="http://127.0.0.1:$RELAY_PORT"
for i in {1..30}; do
    CODE=$(curl -s -o /dev/null -w "%{http_code}" --max-time 2 "$RELAY_URL/health" 2>/dev/null || echo "000")
    if [ "$CODE" == "200" ]; then break; fi
    sleep 1
done
echo -e "${GREEN}Relay ready at $RELAY_URL${NC}"

# ============================================================
# START HOME SERVER
# ============================================================
echo -e "${CYAN}=== Starting Home Server ===${NC}"
mkdir -p "$TEST_DIR/images"
mkdir -p "$TEST_DIR/videos"

# Note: We must register a user on the relay first so the home server can login
echo "Registering home server user on relay..."
curl -s -X POST "$RELAY_URL/api/auth/register" \
    -H "Content-Type: application/json" \
    -d '{"username":"homeserver","password":"password123","email":"home@test.local"}' > /dev/null

cat > "$TEST_DIR/config.yaml" <<EOF
database_url: "postgres://reminisce:reminisce@127.0.0.1:$PG_PORT/reminisce"
geotagging_database_url: "postgres://postgres:postgres@localhost:5435/geotagging_db"
api_secret_key: "relay-test-secret"
images_dir: "$TEST_DIR/images"
videos_dir: "$TEST_DIR/videos"
relay_url: "$RELAY_URL"
relay_username: "homeserver"
relay_password: "password123"
port: $HOME_PORT
EOF

RUST_LOG="info" $HOME_BINARY "$TEST_DIR/config.yaml" > "$TEST_DIR/home.log" 2>&1 &

for i in {1..30}; do
    CODE=$(curl -s -o /dev/null -w "%{http_code}" --max-time 2 "http://127.0.0.1:$HOME_PORT/ping" 2>/dev/null || echo "000")
    if [ "$CODE" == "200" ]; then break; fi
    sleep 1
done
echo -e "${GREEN}Home Server ready at http://127.0.0.1:$HOME_PORT${NC}"

# ============================================================
# TEST 1: WebSocket Tunnel
# ============================================================
echo ""
echo -e "${CYAN}=== Test 1: WebSocket tunnel connection ===${NC}"
sleep 5

if grep -q "Connected to relay WebSocket tunnel" "$TEST_DIR/home.log"; then
    check_pass "Home Server established WebSocket tunnel to Relay"
else
    check_fail "Home Server failed to connect to Relay WebSocket"
    exit 1
fi

# ============================================================
# TEST 2: Streaming Upload (Relay Proxy)
# ============================================================
echo ""
echo -e "${CYAN}=== Test 2: Streaming Upload (Relay Proxy) ===${NC}"

# Create a 5MB file
echo "Generating 5MB test payload..."
dd if=/dev/urandom bs=1M count=5 of=large_test_payload.bin 2>/dev/null

# Get JWT from Relay for the "homeserver" user
LOGIN_RESP=$(curl -s -X POST "$RELAY_URL/api/auth/login" \
    -H "Content-Type: application/json" \
    -d '{"username":"homeserver","password":"password123"}')
TOKEN=$(echo "$LOGIN_RESP" | grep -oP '"token":"\K[^"]+' | head -1)

if [ -z "$TOKEN" ]; then
    check_fail "Failed to get JWT token from relay"
    exit 1
fi

# Upload via Relay Proxy: POST /api/myproxy/api/upload_image
echo "Uploading 5MB via relay proxy..."
UPLOAD_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
    "$RELAY_URL/api/myproxy/api/upload_image" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Cookie: X-Device-ID=e2e-device" \
    -F "image=@large_test_payload.bin" \
    -F "hash=e2e-streaming-hash" \
    -F "name=streaming_test.jpg" \
    --no-buffer \
    --max-time 60)

if [ "$UPLOAD_CODE" == "200" ]; then
    check_pass "Successfully uploaded 5MB file via streaming relay proxy"
else
    check_fail "Failed to upload file via relay proxy (HTTP $UPLOAD_CODE)"
    echo -e "${YELLOW}Relay Logs:${NC}"
    tail -n 20 "$TEST_DIR/relay.log"
    echo -e "${YELLOW}Home Server Logs:${NC}"
    tail -n 20 "$TEST_DIR/home.log"
fi

# ============================================================
# TEST 3: Streaming Download (Relay Proxy)
# ============================================================
echo ""
echo -e "${CYAN}=== Test 3: Streaming Download (Relay Proxy) ===${NC}"

# Download the file we just uploaded
echo "Downloading file via relay proxy..."
DOWNLOAD_CODE=$(curl -s -o downloaded_payload.bin -w "%{http_code}" \
    "$RELAY_URL/api/myproxy/api/image/e2e-streaming-hash" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Cookie: X-Device-ID=e2e-device" \
    --no-buffer \
    --max-time 60)

if [ "$DOWNLOAD_CODE" == "200" ]; then
    # Compare hashes
    ORIG_HASH=$(sha256sum large_test_payload.bin | cut -d' ' -f1)
    DL_HASH=$(sha256sum downloaded_payload.bin | cut -d' ' -f1)
    
    if [ "$ORIG_HASH" == "$DL_HASH" ]; then
        check_pass "Successfully downloaded 5MB file via relay proxy - content matches"
    else
        check_fail "Downloaded file content does not match original!"
    fi
else
    check_fail "Failed to download file via relay proxy (HTTP $DOWNLOAD_CODE)"
fi

# ============================================================
# SUMMARY
# ============================================================
echo ""
echo -e "${CYAN}========================================${NC}"
echo -e "${CYAN}  Streaming Relay Proxy E2E Test${NC}"
echo -e "${CYAN}========================================${NC}"
echo -e "  ${GREEN}Passed: $PASS${NC}"
echo -e "  ${RED}Failed: $FAIL${NC}"
echo ""

if [ $FAIL -gt 0 ]; then
    exit 1
fi
