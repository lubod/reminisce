#!/bin/bash
set -e

# Data directory
PGDATA="/var/lib/postgresql/data"

echo "Initializing database..."
# Initialize DB if empty
if [ -z "$(ls -A \"$PGDATA\" )" ]; then
    chown postgres:postgres "$PGDATA"
    su postgres -c "initdb -D $PGDATA --encoding=UTF8 --locale=C.UTF-8"
fi

echo "Starting Postgres..."
# Start Postgres manually as postgres user
su postgres -c "pg_ctl -D $PGDATA -w start"

echo "Setting up database user and name..."
# Create user and db if they don't exist (idempotent)
su postgres -c "psql -c \"CREATE USER postgres WITH SUPERUSER PASSWORD 'postgres';\" || true"
su postgres -c "psql -c \"CREATE DATABASE geotagging_db OWNER postgres;\" || true"

echo "Creating PostGIS extension..."
su postgres -c "psql -d geotagging_db -c 'CREATE EXTENSION IF NOT EXISTS postgis;'"
su postgres -c "psql -d geotagging_db -c 'CREATE EXTENSION IF NOT EXISTS hstore;'"

echo "Filtering OSM data with osmium-tool..."
# Default to /data/input.pbf if provided, otherwise fail
if [ ! -f /data/input.pbf ]; then
    echo "Error: /data/input.pbf not found. Please mount volume with PBF file."
    # Stop before exit
    su postgres -c "pg_ctl -D $PGDATA stop"
    exit 1
fi

# Filter PBF to only include administrative boundaries (ways and relations)
# Writing to /tmp to avoid using space on the possibly full external drive
FILTERED_PBF="/tmp/filtered.pbf"
echo "Extracting administrative boundaries to $FILTERED_PBF..."
osmium tags-filter /data/input.pbf w/boundary=administrative r/boundary=administrative -o "$FILTERED_PBF" --overwrite

echo "Importing filtered data with osm2pgsql..."
# Run osm2pgsql on filtered data
# -O flex: Use the flexible backend with Lua script
# -S /app/process_osm.lua: Our filtering script
# -d geotagging_db: Target database
# --slim: Store temporary data in DB (needed for relations)
# --drop: Drop temporary tables after import
# --cache 24000: Use 24GB of RAM for cache (optimized for 48GB RAM)
# Running as postgres user to ensure socket access
su postgres -c "osm2pgsql -O flex -S /app/process_osm.lua \
    -d geotagging_db \
    --slim --drop \
    --cache 24000 \
    $FILTERED_PBF"

# Clean up filtered file
rm "$FILTERED_PBF"

echo "Running optimizations and simplifications..."
su postgres -c "psql -d geotagging_db -f /app/optimize.sql"

echo "Stopping Postgres..."
su postgres -c "pg_ctl -D $PGDATA -m fast stop"

echo "Build complete. Data is in /var/lib/postgresql/data"