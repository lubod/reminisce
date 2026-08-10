#!/usr/bin/env bash
#
# Drop leftover Reminisce dev test databases (test_<uuid>).
#
# Test teardown is best-effort, so these databases accumulate over time and
# bloat the Postgres data volume. A bloated data directory makes crash recovery
# take tens of minutes (each restart fsyncs the whole dir). Run this before test
# gates and from cron to keep the volume small (a few GB at most between runs).
#
# Usage: scripts/cleanup-test-dbs.sh [--dry-run]
set -euo pipefail

DB="${POSTGRES_TEST_DB_CONTAINER:-reminisce-dev-db}"
DRY=0
[ "${1:-}" = "--dry-run" ] && DRY=1

list="$(docker exec -i "$DB" psql -U postgres -t -A \
    -c "SELECT datname FROM pg_database WHERE datname LIKE 'test\_%';" 2>/dev/null || true)"

count=0
while IFS= read -r db; do
    [ -n "$db" ] || continue
    count=$((count + 1))
    if [ "$DRY" = "1" ]; then
        echo "would drop: $db"
        continue
    fi
    {
        echo "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '$db' AND pid <> pg_backend_pid();"
        echo "DROP DATABASE IF EXISTS \"$db\";"
    } | docker exec -i "$DB" psql -U postgres -q -v ON_ERROR_STOP=0 -f - >/dev/null 2>&1 || true
done <<< "$list"

if [ "$DRY" = "1" ]; then
    echo "cleanup-test-dbs: $count leftover test database(s) (dry-run)"
else
    echo "cleanup-test-dbs: dropped $count leftover test database(s)"
fi