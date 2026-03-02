#!/bin/bash
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "========================================"
echo "Reminisce Installation Script"
echo "========================================"
echo ""

# Ask for installation directory
print_info() {
    echo "INFO: $1"
}

echo "Where do you want to install Reminisce?"
echo "This will create a 'reminisce' directory with all configuration and data."
echo -n "Installation directory [default: current directory]: "
read INSTALL_BASE_DIR

# Set default if empty
if [ -z "$INSTALL_BASE_DIR" ]; then
    INSTALL_BASE_DIR="."
fi

# Convert to absolute path
INSTALL_BASE_DIR=$(cd "$INSTALL_BASE_DIR" 2>/dev/null && pwd) || INSTALL_BASE_DIR="$(pwd)"

# Create main reminisce directory
REMINISCE_DIR="$INSTALL_BASE_DIR/reminisce"
echo ""
echo "Installation directory: $REMINISCE_DIR"
echo ""

# Function to print colored messages
print_error() {
    echo -e "${RED}ERROR: $1${NC}"
}

print_success() {
    echo -e "${GREEN}SUCCESS: $1${NC}"
}

print_warning() {
    echo -e "${YELLOW}WARNING: $1${NC}"
}

print_info() {
    echo "INFO: $1"
}

# Check if Docker is installed
print_info "Checking if Docker is installed..."
if ! command -v docker &> /dev/null; then
    print_error "Docker is not installed!"
    echo "Please install Docker first:"
    echo "  - Visit: https://docs.docker.com/get-docker/"
    echo "  - Or run: curl -fsSL https://get.docker.com | sh"
    exit 1
fi
print_success "Docker is installed ($(docker --version))"

# Check if Docker Compose is installed
print_info "Checking if Docker Compose is installed..."
if ! docker compose version &> /dev/null; then
    print_error "Docker Compose is not installed!"
    echo "Please install Docker Compose:"
    echo "  - Visit: https://docs.docker.com/compose/install/"
    exit 1
fi
print_success "Docker Compose is installed ($(docker compose version))"

# Check if Docker daemon is running
print_info "Checking if Docker daemon is running..."
if ! docker info &> /dev/null; then
    print_error "Docker daemon is not running!"
    echo "Please start Docker first:"
    echo "  - Linux: sudo systemctl start docker"
    echo "  - Mac/Windows: Start Docker Desktop"
    exit 1
fi
print_success "Docker daemon is running"

echo ""
echo "========================================"
echo "Creating Directory Structure"
echo "========================================"
echo ""

# Create main reminisce directory
print_info "Creating reminisce directory structure..."
mkdir -p "$REMINISCE_DIR"
cd "$REMINISCE_DIR"

# Create .env file with current user's UID/GID
print_info "Creating .env file for user permissions..."
echo "DOCKER_UID=$(id -u)" > .env
echo "DOCKER_GID=$(id -g)" >> .env
print_success ".env file created"

# Create storage directories
print_info "Creating storage directories..."
mkdir -p "./uploaded_images"
mkdir -p "./uploaded_videos"
mkdir -p "./backups"
mkdir -p "./iroh_data"
mkdir -p "./data"
print_success "Storage directories created at $REMINISCE_DIR"

# Create models directory for AI service
print_info "Creating models directory for AI service..."
mkdir -p "./ai/models"
print_success "Models directory created at $REMINISCE_DIR/ai/models"

# Create observability directory
print_info "Creating observability directory..."
mkdir -p "./observability"
print_success "Observability directory created at $REMINISCE_DIR/observability"

echo ""
echo "========================================"
echo "Creating configuration files..."
echo "========================================"
echo ""

# Create docker-compose.yml
print_info "Creating docker-compose.yml in $REMINISCE_DIR..."
cat > "$REMINISCE_DIR/docker-compose.yml" << 'EOF'
# Production setup - Generated from docker-compose.yml
# Usage: docker compose up -d

# Production setup
# Usage: docker compose up -d
# For development, use: docker compose -f docker-compose-dev.yml up -d

services:
  postgres:
    image: lubod/reminisce-postgres:latest
    container_name: reminisce-postgres
    environment:
      POSTGRES_DB: reminisce_db
      POSTGRES_USER: postgres
      POSTGRES_PASSWORD: postgres
    volumes:
      - postgres_data:/var/lib/postgresql/data
      - ./init.sql:/docker-entrypoint-initdb.d/init.sql
    restart: unless-stopped
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres"]
      interval: 10s
      timeout: 5s
      retries: 5

  geotagging-db:
    image: lubod/geodb:latest
    container_name: reminisce-geotagging
    environment:
      POSTGRES_DB: geotagging_db
      POSTGRES_USER: postgres
      POSTGRES_PASSWORD: postgres
    volumes:
      - geotagging_data:/var/lib/postgresql/data
    restart: unless-stopped
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres"]
      interval: 10s
      timeout: 5s
      retries: 5

  ai-server:
    image: lubod/reminisce-ai-server:latest
    container_name: reminisce-ai-server
    expose:
      - "8081"
    volumes:
      - ai_model_cache:/root/.cache/huggingface
      - face_model_cache:/root/.insightface
    devices:
      # GPU access (works with Intel, AMD, and NVIDIA via /dev/dri)
      # Will fall back to CPU if no GPU is available
      - /dev/dri:/dev/dri
    restart: unless-stopped
    healthcheck:
      test: ["CMD-SHELL", "curl -f http://localhost:8081/health || exit 1"]
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 60s

  tempo:
    image: grafana/tempo:latest
    container_name: reminisce-tempo
    command: [ "-config.file=/etc/tempo.yaml" ]
    volumes:
      - ./observability/tempo.yaml:/etc/tempo.yaml:ro
      - tempo_data:/var/tempo
    expose:
      - "4317"
      - "3200"
    restart: unless-stopped

  grafana:
    image: grafana/grafana:latest
    container_name: reminisce-grafana
    volumes:
      - ./observability/grafana-datasources.yaml:/etc/grafana/provisioning/datasources/datasources.yaml:ro
      - grafana_data:/var/lib/grafana
    environment:
      - GF_AUTH_ANONYMOUS_ENABLED=true
      - GF_AUTH_ANONYMOUS_ORG_ROLE=Admin
      - GF_AUTH_DISABLE_LOGIN_FORM=true
    ports:
      - "3000:3000"
    restart: unless-stopped
    depends_on:
      - tempo

  reminisce:
    image: lubod/reminisce:latest
    container_name: reminisce
    expose:
      - "8080"
      - "8443"
    ports:
      - "4001:4001"
    volumes:
      - ./uploaded_images:/app/uploaded_images
      - ./uploaded_videos:/app/uploaded_videos
      - ./backups:/app/backups
      - ./iroh_data:/app/iroh_data
      - ./data:/app/data
      - ./config.yaml:/app/config.yaml:ro
    environment:
      - RUST_LOG=info
      - OTEL_EXPORTER_OTLP_ENDPOINT=http://tempo:4317
      - APP_VERSION=0.1.0
      - ENVIRONMENT=production
      - OTEL_TRACE_SAMPLE_RATE=1.0
    restart: unless-stopped
    user: "1000:1000"
    depends_on:
      postgres:
        condition: service_healthy
      ai-server:
        condition: service_healthy
      tempo:
        condition: service_started

  client:
    image: lubod/reminisce-client:latest
    container_name: reminisce-client
    ports:
      - "28080:80"
      - "28443:443"
    volumes:
      - client_ssl:/etc/nginx/ssl
    restart: unless-stopped
    depends_on:
      - reminisce

volumes:
  postgres_data:
  geotagging_data:
  ai_model_cache:
  face_model_cache:
  tempo_data:
  grafana_data:
  client_ssl:

EOF
print_success "docker-compose.yml created"

# Create init.sql
print_info "Creating init.sql in $REMINISCE_DIR..."
cat > "$REMINISCE_DIR/init.sql" << 'EOF'
-- This SQL file will be executed automatically when the PostgreSQL container starts
-- if the database is empty, which is perfect for initializing our schema.

-- Enable PostGIS extension for geospatial support
CREATE EXTENSION IF NOT EXISTS postgis;

-- Enable pgvector extension for vector similarity search (AI-powered semantic search)
CREATE EXTENSION IF NOT EXISTS vector;

-- Users table for authentication (must be created first due to foreign key constraints)
CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    username VARCHAR(255) UNIQUE NOT NULL,
    email VARCHAR(255) UNIQUE NOT NULL,
    password_hash VARCHAR(255) NOT NULL,
    role VARCHAR(50) NOT NULL DEFAULT 'user',
    is_active BOOLEAN DEFAULT TRUE,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    last_login_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_users_username ON users(username);
CREATE INDEX IF NOT EXISTS idx_users_email ON users(email);

-- Insert default admin user
-- Password: "admin123" (CHANGE IMMEDIATELY AFTER FIRST LOGIN!)
-- Hash generated using Argon2::default() with random salt
INSERT INTO users (username, email, password_hash, role)
VALUES (
    'admin',
    'admin@localhost',
    '$argon2id$v=19$m=19456,t=2,p=1$ykODG4Kjv3ZOijtRLuNlFA$+6QnBbvOF+uWMm/po/O6mEZc9I9sZ/VBzi0fnp95ZnM',
    'admin'
)
ON CONFLICT (username) DO NOTHING;

-- Insert test user for integration tests (with specific UUID)
INSERT INTO users (id, username, email, password_hash, role)
VALUES (
    '550e8400-e29b-41d4-a716-446655440000',
    'test-user',
    'test@localhost',
    '$argon2id$v=19$m=19456,t=2,p=1$ykODG4Kjv3ZOijtRLuNlFA$+6QnBbvOF+uWMm/po/O6mEZc9I9sZ/VBzi0fnp95ZnM',
    'admin'
)
ON CONFLICT (id) DO NOTHING;

-- Create the tables if they don't exist (same as schema.sql)
CREATE TABLE IF NOT EXISTS images (
    deviceid VARCHAR(255) NOT NULL,
    hash VARCHAR(255) NOT NULL,
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    type VARCHAR(255),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    added_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    name VARCHAR(255) NOT NULL,
    ext VARCHAR(10) NOT NULL,
    exif TEXT,
    has_thumbnail BOOLEAN DEFAULT FALSE,
    last_verified_at TIMESTAMPTZ,
    verification_status INTEGER NOT NULL DEFAULT 0, -- 0: not verified/pending, 1: OK/verified, -1: NOK/failed
    location GEOGRAPHY(POINT, 4326), -- WGS84 coordinate system for GPS data
    place TEXT,
    description TEXT, -- AI-generated description of the image
    embedding vector(1152), -- SigLIP image embedding vector (1152-dimensional) for semantic search
    embedding_generated_at TIMESTAMPTZ, -- Timestamp when embedding was generated
    face_detection_completed_at TIMESTAMPTZ, -- Timestamp when face detection was completed (even if 0 faces found)
    deleted_at TIMESTAMPTZ, -- Timestamp for soft deletion
    p2p_synced_at TIMESTAMPTZ, -- Timestamp when this media file was last synced to P2P network. NULL means needs sync.
    p2p_shard_hash VARCHAR(255), -- Root hash or manifest hash of the object in P2P network (Blake3).
    p2p_encryption_key BYTEA, -- 32-byte encryption key used for sharding (needed for re-sharding during rebalance)
    p2p_encrypted_size INTEGER, -- Size of the encrypted blob before erasure coding (needed for reconstruction)
    PRIMARY KEY (deviceid, hash)
);

CREATE INDEX IF NOT EXISTS idx_deviceid_type_created_at ON images(deviceid, type, created_at DESC);
-- Index for soft delete filtering
CREATE INDEX IF NOT EXISTS idx_images_deleted_at ON images(deleted_at) WHERE deleted_at IS NULL;
-- Index for verification status
CREATE INDEX IF NOT EXISTS idx_images_verification_status ON images(verification_status);
-- Spatial index for location-based queries (e.g., find images near a location)
CREATE INDEX IF NOT EXISTS idx_images_location ON images USING GIST(location);
-- Index for user_id queries
CREATE INDEX IF NOT EXISTS idx_images_user_id ON images(user_id);
-- HNSW index for fast approximate nearest neighbor search (semantic image search)
-- m=16: number of connections per layer
-- ef_construction=64: build quality (higher = better but slower indexing)
CREATE INDEX IF NOT EXISTS idx_images_embedding_hnsw ON images
USING hnsw (embedding vector_cosine_ops)
WITH (m = 16, ef_construction = 64);
-- Index to track images without embeddings for backfill worker
CREATE INDEX IF NOT EXISTS idx_images_embedding_status ON images(embedding_generated_at)
WHERE embedding_generated_at IS NULL;
-- Index for fast hash lookups (used in get_image, toggle_star, metadata queries)
CREATE INDEX IF NOT EXISTS idx_images_hash ON images(hash);
-- Partial index for thumbnail queries (only index rows with thumbnails)
CREATE INDEX IF NOT EXISTS idx_images_has_thumbnail ON images(has_thumbnail) WHERE has_thumbnail = true;
-- Full-text search index on description and name for fast text-based search
-- Uses GIN index with english text search configuration
-- This enables queries like: WHERE to_tsvector('english', description) @@ plainto_tsquery('english', 'sunset beach')
CREATE INDEX IF NOT EXISTS idx_images_description_fts ON images
USING GIN (to_tsvector('english', COALESCE(description, '') || ' ' || COALESCE(name, '')));
-- Partial index for existence checks (indexes id, not the text value, to avoid btree row size limits)
CREATE INDEX IF NOT EXISTS idx_images_description_exists ON images(hash) WHERE description IS NOT NULL AND description != '';
-- Index to find images that haven't had face detection run yet
CREATE INDEX IF NOT EXISTS idx_images_face_detection_pending ON images(face_detection_completed_at) WHERE face_detection_completed_at IS NULL;
-- Index for finding unsynced images efficiently (P2P media replication)
CREATE INDEX IF NOT EXISTS idx_images_need_sync ON images(created_at) WHERE p2p_synced_at IS NULL;
-- Index for looking up P2P synced status
CREATE INDEX IF NOT EXISTS idx_images_p2p_synced ON images(p2p_synced_at);

-- Starred images table for per-user image starring (cross-device)
CREATE TABLE IF NOT EXISTS starred_images (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    hash VARCHAR(255) NOT NULL,
    starred_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, hash)
);

CREATE INDEX IF NOT EXISTS idx_starred_images_user_id ON starred_images(user_id);
CREATE INDEX IF NOT EXISTS idx_starred_images_hash ON starred_images(hash);
CREATE INDEX IF NOT EXISTS idx_starred_images_starred_at ON starred_images(starred_at DESC);

CREATE TABLE IF NOT EXISTS starred_videos (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    hash VARCHAR(255) NOT NULL,
    starred_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, hash)
);

CREATE INDEX IF NOT EXISTS idx_starred_videos_user_id ON starred_videos(user_id);
CREATE INDEX IF NOT EXISTS idx_starred_videos_hash ON starred_videos(hash);
CREATE INDEX IF NOT EXISTS idx_starred_videos_starred_at ON starred_videos(starred_at DESC);

CREATE TABLE IF NOT EXISTS videos (
    deviceid VARCHAR(255) NOT NULL,
    hash VARCHAR(255) NOT NULL,
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    type VARCHAR(255),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    added_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    name VARCHAR(255) NOT NULL,
    ext VARCHAR(10) NOT NULL,
    metadata TEXT,
    has_thumbnail BOOLEAN DEFAULT FALSE,
    last_verified_at TIMESTAMPTZ,
    verification_status INTEGER NOT NULL DEFAULT 0, -- 0: not verified/pending, 1: OK/verified, -1: NOK/failed
    description TEXT, -- AI-generated description of the video
    deleted_at TIMESTAMPTZ, -- Timestamp for soft deletion
    p2p_synced_at TIMESTAMPTZ, -- Timestamp when this media file was last synced to P2P network. NULL means needs sync.
    p2p_shard_hash VARCHAR(255), -- Root hash or manifest hash of the object in P2P network (Blake3).
    p2p_encryption_key BYTEA, -- 32-byte encryption key used for sharding (needed for re-sharding during rebalance)
    p2p_encrypted_size INTEGER, -- Size of the encrypted blob before erasure coding (needed for reconstruction)
    PRIMARY KEY (deviceid, hash)
);

CREATE INDEX IF NOT EXISTS idx_video_deviceid_type_created_at ON videos(deviceid, type, created_at DESC);
-- Index for soft delete filtering
CREATE INDEX IF NOT EXISTS idx_videos_deleted_at ON videos(deleted_at) WHERE deleted_at IS NULL;
-- Index for verification status
CREATE INDEX IF NOT EXISTS idx_videos_verification_status ON videos(verification_status);
-- Index for user_id queries
CREATE INDEX IF NOT EXISTS idx_videos_user_id ON videos(user_id);
-- Index for fast hash lookups (used in get_video, existence checks)
CREATE INDEX IF NOT EXISTS idx_videos_hash ON videos(hash);
-- Partial index for thumbnail queries (only index rows with thumbnails)
CREATE INDEX IF NOT EXISTS idx_videos_has_thumbnail ON videos(has_thumbnail) WHERE has_thumbnail = true;
-- Index for finding unsynced videos efficiently (P2P media replication)
CREATE INDEX IF NOT EXISTS idx_videos_need_sync ON videos(created_at) WHERE p2p_synced_at IS NULL;
-- Index for looking up P2P synced status
CREATE INDEX IF NOT EXISTS idx_videos_p2p_synced ON videos(p2p_synced_at);

-- Persons table (face clusters representing individuals)
-- MOVED BEFORE FACES TO RESOLVE CIRCULAR DEPENDENCY
CREATE TABLE IF NOT EXISTS persons (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,

    -- Person name (optional, user-provided)
    name VARCHAR(255),

    -- Representative face embedding (centroid or best face embedding from cluster)
    representative_embedding vector(512),

    -- Representative face (for thumbnail display)
    -- Constraint added later to avoid circular dependency
    representative_face_id BIGINT,

    -- Metadata
    face_count INTEGER NOT NULL DEFAULT 0,  -- Number of faces in this cluster
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Indices for persons table
CREATE INDEX IF NOT EXISTS idx_persons_user_id ON persons(user_id);
CREATE INDEX IF NOT EXISTS idx_persons_name ON persons(name) WHERE name IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_persons_face_count ON persons(face_count DESC);
CREATE INDEX IF NOT EXISTS idx_persons_updated_at ON persons(updated_at DESC);

-- HNSW index for person similarity (for finding similar persons)
CREATE INDEX IF NOT EXISTS idx_persons_embedding_hnsw ON persons
USING hnsw (representative_embedding vector_cosine_ops)
WITH (m = 16, ef_construction = 64);

-- Face detection and recognition tables for person identification
CREATE TABLE IF NOT EXISTS faces (
    id BIGSERIAL PRIMARY KEY,
    image_hash VARCHAR(255) NOT NULL,
    image_deviceid VARCHAR(255) NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,

    -- Face location (bounding box in original image coordinates)
    bbox_x INTEGER NOT NULL,
    bbox_y INTEGER NOT NULL,
    bbox_width INTEGER NOT NULL,
    bbox_height INTEGER NOT NULL,

    -- Face embedding (512-dimensional from InsightFace)
    embedding vector(512) NOT NULL,

    -- Face quality metrics
    confidence REAL NOT NULL,  -- Detection confidence (0.0-1.0)

    -- Person clustering
    person_id BIGINT REFERENCES persons(id) ON DELETE SET NULL,

    -- Metadata
    detected_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Foreign key to images table
    FOREIGN KEY (image_deviceid, image_hash) REFERENCES images(deviceid, hash) ON DELETE CASCADE
);

-- Indices for faces table
CREATE INDEX IF NOT EXISTS idx_faces_image ON faces(image_deviceid, image_hash);
CREATE INDEX IF NOT EXISTS idx_faces_user_id ON faces(user_id);
CREATE INDEX IF NOT EXISTS idx_faces_person_id ON faces(person_id);
CREATE INDEX IF NOT EXISTS idx_faces_detected_at ON faces(detected_at DESC);

-- HNSW index for face similarity search (same config as CLIP: m=16, ef_construction=64)
CREATE INDEX IF NOT EXISTS idx_faces_embedding_hnsw ON faces
USING hnsw (embedding vector_cosine_ops)
WITH (m = 16, ef_construction = 64);

-- Add circular foreign key constraint
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 
        FROM information_schema.table_constraints 
        WHERE constraint_name = 'fk_persons_representative_face' 
        AND table_name = 'persons'
    ) THEN
        ALTER TABLE persons
        ADD CONSTRAINT fk_persons_representative_face
        FOREIGN KEY (representative_face_id)
        REFERENCES faces(id)
        ON DELETE SET NULL;
    END IF;
END $$;

-- AI processing settings table (per-user settings)
-- This stores runtime-configurable AI processing preferences
CREATE TABLE IF NOT EXISTS ai_settings (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,

    -- AI description generation (using vision models)
    enable_ai_descriptions BOOLEAN NOT NULL DEFAULT TRUE,

    -- CLIP embeddings for semantic search
    enable_embeddings BOOLEAN NOT NULL DEFAULT TRUE,
    embedding_parallel_count INTEGER NOT NULL DEFAULT 10,

    -- Face detection and recognition
    enable_face_detection BOOLEAN NOT NULL DEFAULT TRUE,
    face_detection_parallel_count INTEGER NOT NULL DEFAULT 3,

    -- Metadata
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_ai_settings_user_id ON ai_settings(user_id);

-- Labels table for organizing media
CREATE TABLE IF NOT EXISTS labels (
    id SERIAL PRIMARY KEY,
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    color VARCHAR(7) DEFAULT '#808080', -- Hex color code
    created_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE(user_id, name)
);

CREATE INDEX IF NOT EXISTS idx_labels_user_id ON labels(user_id);

-- Many-to-many relationship between images and labels
CREATE TABLE IF NOT EXISTS image_labels (
    image_hash VARCHAR(255) NOT NULL,
    image_deviceid VARCHAR(255) NOT NULL,
    label_id INTEGER REFERENCES labels(id) ON DELETE CASCADE,
    PRIMARY KEY (image_hash, image_deviceid, label_id),
    FOREIGN KEY (image_deviceid, image_hash) REFERENCES images(deviceid, hash) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_image_labels_label_id ON image_labels(label_id);
CREATE INDEX IF NOT EXISTS idx_image_labels_image ON image_labels(image_deviceid, image_hash);

-- Many-to-many relationship between videos and labels
CREATE TABLE IF NOT EXISTS video_labels (
    video_hash VARCHAR(255) NOT NULL,
    video_deviceid VARCHAR(255) NOT NULL,
    label_id INTEGER REFERENCES labels(id) ON DELETE CASCADE,
    PRIMARY KEY (video_hash, video_deviceid, label_id),
    FOREIGN KEY (video_deviceid, video_hash) REFERENCES videos(deviceid, hash) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_video_labels_label_id ON video_labels(label_id);
CREATE INDEX IF NOT EXISTS idx_video_labels_video ON video_labels(video_deviceid, video_hash);

-- Note: admin_boundaries table is now in a separate geotagging database
-- See init.geotagging.sql for the geotagging database schemaEOF
print_success "init.sql created"

# Create config.yaml if it doesn't exist
if [ ! -f "$REMINISCE_DIR/config.yaml" ]; then
    print_info "Creating config.yaml in $REMINISCE_DIR..."
    cat > "$REMINISCE_DIR/config.yaml" << 'EOF'
# Database connection string - for Docker setup
database_url: "postgres://postgres:postgres@postgres:5432/reminisce_db"

# Geotagging database (for reverse geocoding)
geotagging_database_url: "postgres://postgres:postgres@geotagging-db:5432/geotagging_db"

# Secret key for API authentication
# IMPORTANT: Change this to a strong random secret!
api_secret_key: "CHANGE_THIS_TO_A_STRONG_SECRET_KEY"

# Directory for storing uploaded images
images_dir: "uploaded_images"

# Directory for storing uploaded videos
videos_dir: "uploaded_videos"

# Geocoding configuration
enable_local_geocoding: true
enable_external_geocoding_fallback: true

# AI service URL for image embeddings and semantic search (CLIP model)
ai_service_url: "http://ai-server:8081"

# Face detection service URL (Consolidated into ai-server)
face_service_url: "http://ai-server:8081"

EOF
    print_success "config.yaml created at $REMINISCE_DIR/config.yaml"
    print_warning "Please edit $REMINISCE_DIR/config.yaml and set your api_secret_key!"
else
    print_info "config.yaml already exists, skipping..."
fi

# Create Tempo configuration
print_info "Creating Tempo configuration..."
cat > "$REMINISCE_DIR/observability/tempo.yaml" << 'EOF'
server:
  http_listen_port: 3200

distributor:
  receivers:
    otlp:
      protocols:
        grpc:
          endpoint: "0.0.0.0:4317"
        http:
          endpoint: "0.0.0.0:4318"

ingester:
  trace_idle_period: 10s
  max_block_bytes: 1_000_000
  max_block_duration: 5m

compactor:
  compaction:
    compaction_window: 1h
    max_block_bytes: 100_000_000
    block_retention: 336h
    compacted_block_retention: 24h

storage:
  trace:
    backend: local
    wal:
      path: /var/tempo/wal
    local:
      path: /var/tempo/blocks

EOF
print_success "Tempo configuration created"

# Create Grafana datasources configuration
print_info "Creating Grafana datasources configuration..."
cat > "$REMINISCE_DIR/observability/grafana-datasources.yaml" << 'EOF'
apiVersion: 1

datasources:
  - name: Tempo
    type: tempo
    access: proxy
    orgId: 1
    url: http://tempo:3200
    basicAuth: false
    isDefault: true
    version: 1
    editable: false
    uid: tempo

EOF
print_success "Grafana datasources configuration created"

echo ""
echo "========================================"
echo "Copying project files..."
echo "========================================"
echo ""

# Get the script's directory (where install.sh is located - the source repo)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Copy ai directory if it exists in the source (optional - for customization only)
if [ -d "$SCRIPT_DIR/ai" ]; then
    print_info "Copying AI service source files (optional) to $REMINISCE_DIR..."
    cp -r "$SCRIPT_DIR/ai" "$REMINISCE_DIR/"
    print_success "AI service source files copied (for customization if needed)"
else
    print_info "AI directory not found. Installation will use pre-built Docker images only."
fi

print_success "All required files are in place"


echo ""
echo "========================================"
echo "Pulling latest Docker images..."
echo "========================================"
echo ""

print_info "Pulling PostgreSQL database image (with PostGIS + pgvector)..."
docker pull lubod/reminisce-postgres:latest
print_success "PostgreSQL database image pulled"

print_info "Pulling reminisce image..."
docker pull lubod/reminisce:latest
print_success "Reminisce image pulled"

print_info "Pulling geotagging database image..."
docker pull lubod/geodb:latest
print_success "Geotagging database image pulled"

print_info "Pulling AI server image (Unified: SigLIP + Florence-2 + InsightFace)..."
docker pull lubod/reminisce-ai-server:latest
print_success "AI server image pulled"

print_info "Pulling client image..."
docker pull lubod/reminisce-client:latest
print_success "Client image pulled"

echo ""
echo "========================================"
echo "Starting services..."
echo "========================================"
echo ""

# Change to reminisce directory and start services
cd "$REMINISCE_DIR"
docker compose up -d

echo ""
echo "========================================"
echo "Installation Complete!"
echo "========================================"
echo ""

# Wait a bit for services to start
sleep 10

# Check if services are running
if docker compose ps | grep -q "reminisce.*running"; then
    print_success "Reminisce is running!"
else
    print_warning "Reminisce may not be running correctly"
    echo "Check logs with: docker compose logs reminisce"
fi

if docker compose ps | grep -q "reminisce-postgres.*running"; then
    print_success "PostgreSQL database is running!"
else
    print_warning "PostgreSQL may not be running correctly"
    echo "Check logs with: docker compose logs postgres"
fi

if docker compose ps | grep -q "reminisce-geotagging.*running"; then
    print_success "Geotagging database is running!"
else
    print_warning "Geotagging database may not be running correctly"
    echo "Check logs with: docker compose logs geotagging-db"
fi

if docker compose ps | grep -q "reminisce-ai-server.*running"; then
    print_success "Unified AI service (CLIP + Vision + Face) is running!"
else
    print_warning "AI service may not be running correctly"
    echo "Check logs with: docker compose logs ai-server"
fi

if docker compose ps | grep -q "client.*running"; then
    print_success "Client web server is running!"
else
    print_warning "Client web server may not be running correctly"
    echo "Check logs with: docker compose logs client"
fi

if docker compose ps | grep -q "reminisce-tempo.*running"; then
    print_success "Tempo tracing service is running!"
else
    print_warning "Tempo may not be running correctly"
    echo "Check logs with: docker compose logs tempo"
fi

if docker compose ps | grep -q "reminisce-grafana.*running"; then
    print_success "Grafana monitoring dashboard is running!"
else
    print_warning "Grafana may not be running correctly"
    echo "Check logs with: docker compose logs grafana"
fi

echo ""
echo "========================================"
echo "Installation Complete!"
echo "========================================"
echo ""
echo "Installation directory: $REMINISCE_DIR"
echo ""
echo "Services are accessible at:"
echo "  - Web Client (HTTPS): https://localhost:28443"
echo "  - Web Client (HTTP): http://localhost:28080"
echo "  - API (HTTPS): https://localhost:28443/api/"
echo "  - API (HTTP): http://localhost:28080/api/"
echo "  - Swagger UI: https://localhost:28443/api/swagger-ui/"
echo "  - Grafana: http://localhost:3000"
echo ""
echo "Features:"
echo "  ✓ Semantic image search powered by SigLIP (1152-dimensional embeddings)"
echo "  ✓ Face detection and person clustering (InsightFace with 512-dim embeddings)"
echo "  ✓ Fast similarity search using pgvector with HNSW index"
echo "  ✓ Reverse geocoding with PostGIS"
echo "  ✓ Multi-user support with authentication"
echo "  ✓ Image starring/favorites and labeling"
echo "  ✓ GPU acceleration for AI services (auto-detected)"
echo "  ✓ P2P backup with encryption and erasure coding"
echo "  ✓ OpenTelemetry observability with Tempo tracing"
echo "  ✓ Grafana dashboards for monitoring"
echo ""
echo "Note: All traffic flows through nginx. Reminisce is not directly accessible."
echo ""
echo "Useful commands (run from $REMINISCE_DIR):"
echo "  - View logs: cd $REMINISCE_DIR && docker compose logs -f"
echo "  - Stop services: cd $REMINISCE_DIR && docker compose down"
echo "  - Restart services: cd $REMINISCE_DIR && docker compose restart"
echo ""
echo "Directory structure:"
echo "  - Config: $REMINISCE_DIR/config.yaml"
echo "  - Docker Compose: $REMINISCE_DIR/docker-compose.yml"
echo "  - Images: $REMINISCE_DIR/uploaded_images"
echo "  - Videos: $REMINISCE_DIR/uploaded_videos"
echo "  - P2P Backups: $REMINISCE_DIR/backups"
echo "  - Iroh Data: $REMINISCE_DIR/iroh_data"
echo "  - Node Identity: $REMINISCE_DIR/data"
echo ""
print_warning "Don't forget to:"
echo "  1. Edit $REMINISCE_DIR/config.yaml and set a strong api_secret_key"
echo "  2. For production, replace self-signed certificates with real ones"
echo "  3. Restart services after changes: cd $REMINISCE_DIR && docker compose restart"
echo ""
echo "GPU Acceleration:"
echo "  GPU support is ENABLED BY DEFAULT for Intel, AMD, and NVIDIA GPUs!"
echo "  The services automatically detect available GPUs via /dev/dri"
echo "  Falls back to CPU if no GPU is detected"
echo "  CLIP (semantic search) runs ~10x faster on GPU"
echo ""
echo "Login credentials:"
echo "  - Username: admin"
echo "  - Password: admin123"
echo "  - IMPORTANT: Change the password after first login!"
echo ""
