#!/bin/bash
set -e

# Change directory to the script's location
cd "$(dirname "$0")"

# Configuration
# Full planet: https://planet.openstreetmap.org/pbf/planet-latest.osm.pbf
DEFAULT_PBF_URL="https://planet.openstreetmap.org/pbf/planet-latest.osm.pbf"

PBF_URL="${1:-$DEFAULT_PBF_URL}"
PBF_FILE="input.pbf"
BUILDER_IMAGE="geodb-builder"
FINAL_IMAGE="lubod/reminisce-geodb:latest"

echo "========================================"
echo "Geotagging DB Builder"
echo "========================================"
echo "Target PBF: $PBF_URL"

# 1. Download Data
if [ ! -f "$PBF_FILE" ]; then
    echo "Downloading PBF file..."
    curl -L -o "$PBF_FILE" "$PBF_URL"
else
    echo "Using existing $PBF_FILE"
fi

# 2. Build Builder Image
echo "Building builder image..."
docker build -t "$BUILDER_IMAGE" -f Dockerfile.builder .

# 3. Run Builder to generate data
echo "Running builder container..."

# Allow overriding the data directory (e.g. for external drives)
# Usage: PGDATA_DIR=/mnt/external/pgdata ./build.sh
TARGET_DATA_DIR="${PGDATA_DIR:-$(pwd)/pgdata}"
echo "Using data directory: $TARGET_DATA_DIR"

# Create directory if it doesn't exist
if [ ! -d "$TARGET_DATA_DIR" ]; then
    mkdir -p "$TARGET_DATA_DIR"
fi

# Ensure permissions (clean up if existing)
if [ -d "$TARGET_DATA_DIR" ]; then
    # Try simple remove of contents if it looks like a failed run (has postgresql.conf)
    if [ -f "$TARGET_DATA_DIR/postgresql.conf" ]; then
         echo "Cleaning up previous run data in $TARGET_DATA_DIR..."
         rm -rf "$TARGET_DATA_DIR"/* || sudo rm -rf "$TARGET_DATA_DIR"/*
    fi
fi
chmod 777 "$TARGET_DATA_DIR"

# Run the builder
# We mount the input PBF file
# We mount the TARGET_DATA_DIR to /var/lib/postgresql/data inside the container
docker run --rm \
    -v "$(pwd)/$PBF_FILE:/data/input.pbf" \
    -v "$TARGET_DATA_DIR:/var/lib/postgresql/data" \
    "$BUILDER_IMAGE"

# 4. Build Final Image
echo "Building final runtime image..."
docker build -t "$FINAL_IMAGE" -f Dockerfile.final .

echo "========================================"
echo "Success! Image $FINAL_IMAGE created."
echo "Size: $(docker images $FINAL_IMAGE --format "{{.Size}}")"
echo "To run: docker run -p 5432:5432 $FINAL_IMAGE"
echo "========================================"
