#!/bin/bash
# Script to generate install.sh from docker-compose.yml and init.sql
# This ensures install.sh stays in sync with the source files

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
OUTPUT_FILE="$SCRIPT_DIR/install.sh"
DOCKER_COMPOSE_FILE="$REPO_ROOT/docker-compose.yml"
INIT_SQL_FILE="$REPO_ROOT/db/init.sql"

# Check if source files exist
if [ ! -f "$DOCKER_COMPOSE_FILE" ]; then
    echo "ERROR: docker-compose.yml not found at $DOCKER_COMPOSE_FILE"
    exit 1
fi

if [ ! -f "$INIT_SQL_FILE" ]; then
    echo "ERROR: db/init.sql not found at $INIT_SQL_FILE"
    exit 1
fi

echo "Generating install.sh..."
echo "  Source: $DOCKER_COMPOSE_FILE"
echo "  Source: $INIT_SQL_FILE"
echo "  Output: $OUTPUT_FILE"

# Generate the install.sh file
cat > "$OUTPUT_FILE" << 'INSTALL_HEADER'
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

echo ""
echo "========================================"
echo "Creating configuration files..."
echo "========================================"
echo ""

# Create docker-compose.yml
print_info "Creating docker-compose.yml in $REMINISCE_DIR..."
cat > "$REMINISCE_DIR/docker-compose.yml" << 'EOF'
INSTALL_HEADER

# Now extract and adapt the docker-compose.yml content
# Remove volumes section and adapt paths for install script
echo "# Production setup - Generated from docker-compose.yml" >> "$OUTPUT_FILE"
echo "# Usage: docker compose up -d" >> "$OUTPUT_FILE"
echo "" >> "$OUTPUT_FILE"

# Process docker-compose.yml and adapt it for install.sh
# - Change named volumes to relative paths
# - Change user from hardcoded to variable
sed -e 's|postgres_data:|./postgres_data:|g' \
    -e 's|geotagging_data:|./geotagging_data:|g' \
    -e 's|clip_model_cache:|./clip_model_cache:|g' \
    -e 's|face_model_cache:|./face_model_cache:|g' \
    -e 's|client_ssl:|./client_ssl:|g' \
    -e 's|user: "1000:1000"|user: "${DOCKER_UID}:${DOCKER_GID}"|g' \
    "$DOCKER_COMPOSE_FILE" | \
    # Remove the volumes section at the end (lines starting with "volumes:" to end)
    sed '/^volumes:$/,$ d' \
    >> "$OUTPUT_FILE"

# Continue with the rest of install.sh
cat >> "$OUTPUT_FILE" << 'INSTALL_MIDDLE'
EOF
print_success "docker-compose.yml created"

# Create init.sql
print_info "Creating init.sql in $REMINISCE_DIR..."
cat > "$REMINISCE_DIR/init.sql" << 'EOF'
INSTALL_MIDDLE

# Append the entire init.sql file
cat "$INIT_SQL_FILE" >> "$OUTPUT_FILE"

# Add the rest of the install script
cat >> "$OUTPUT_FILE" << 'INSTALL_FOOTER'
EOF
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
INSTALL_FOOTER

# Make the generated install.sh executable
chmod +x "$OUTPUT_FILE"

echo ""
echo "✓ Successfully generated $OUTPUT_FILE"
echo ""
echo "Changes made:"
echo "  - Converted named volumes to relative paths (./postgres_data, etc.)"
echo "  - Changed user from '1000:1000' to '\${DOCKER_UID}:\${DOCKER_GID}'"
echo "  - Removed volumes section (not needed with relative paths)"
echo "  - Updated init.sql with latest schema (face detection, labels, etc.)"
echo ""
echo "To test the generated install.sh:"
echo "  1. Copy it to a clean directory"
echo "  2. Run: bash install.sh"
echo ""
