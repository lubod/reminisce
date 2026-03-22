#!/bin/bash
# Custom entrypoint for geotagging database
# Fixes pg_hba.conf and listen_addresses before postgres starts,
# since the data directory is pre-baked into the image.

set -e

PGDATA="${PGDATA:-/var/lib/postgresql/data}"

if [ -f "$PGDATA/pg_hba.conf" ]; then
    # Allow connections from all Docker network addresses
    if ! grep -q "host all all 0.0.0.0/0" "$PGDATA/pg_hba.conf"; then
        echo "host all all 0.0.0.0/0 md5" >> "$PGDATA/pg_hba.conf"
        echo "host all all ::/0 md5"       >> "$PGDATA/pg_hba.conf"
    fi
fi

if [ -f "$PGDATA/postgresql.conf" ]; then
    # Listen on all interfaces, not just localhost
    sed -i "s|^#*\s*listen_addresses\s*=.*|listen_addresses = '*'|" "$PGDATA/postgresql.conf"
fi

exec docker-entrypoint.sh "$@"
