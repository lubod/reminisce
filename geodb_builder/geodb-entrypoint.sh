#!/bin/bash
# Custom entrypoint wrapper for geotagging database
# Ensures pg_hba.conf allows external connections on every container start

set -e

# Function to fix pg_hba.conf after postgres is ready
fix_pg_hba() {
    sleep 5  # Wait for postgres to initialize
    if [ -f /var/lib/postgresql/data/pg_hba.conf ]; then
        if ! grep -q "host all all 0.0.0.0/0 trust" /var/lib/postgresql/data/pg_hba.conf; then
            echo "host all all 0.0.0.0/0 trust" >> /var/lib/postgresql/data/pg_hba.conf
            # Reload postgres configuration
            psql -U postgres -c "SELECT pg_reload_conf();" >/dev/null 2>&1 || true
        fi
    fi
}

# Run pg_hba fix in background
fix_pg_hba &

# Execute the original postgres entrypoint
exec docker-entrypoint.sh "$@"
