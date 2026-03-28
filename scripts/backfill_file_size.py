#!/usr/bin/env python3
"""
Backfill file_size_bytes for existing images and videos that have NULL file size.

Usage:
    python3 scripts/backfill_file_size.py [--limit N] [--dry-run]
"""

import argparse
import os
import psycopg2

DB_URL = os.environ.get("DATABASE_URL", "postgresql://postgres:postgres@localhost:5432/reminisce_db")
IMAGES_DIR = os.environ.get("IMAGES_DIR", "./uploaded_images")
VIDEOS_DIR = os.environ.get("VIDEOS_DIR", "./uploaded_videos")


def get_pending(conn, table, limit):
    with conn.cursor() as cur:
        cur.execute(
            f"SELECT hash, ext, deviceid FROM {table} WHERE file_size_bytes IS NULL AND deleted_at IS NULL LIMIT %s",
            (limit,),
        )
        return cur.fetchall()


def media_path(media_dir, hash_, ext):
    return os.path.join(media_dir, hash_[:2], f"{hash_}.{ext}")


def backfill_table(conn, table, media_dir, limit, dry_run):
    rows = get_pending(conn, table, limit)
    print(f"{table}: {len(rows)} rows with missing file_size_bytes")
    if not rows:
        return

    ok = skip = fail = 0
    for i, (hash_, ext, deviceid) in enumerate(rows):
        path = media_path(media_dir, hash_, ext)
        if not os.path.exists(path):
            print(f"  [{i+1}/{len(rows)}] SKIP {hash_[:12]}.{ext} — file not found")
            skip += 1
            continue

        size = os.path.getsize(path)
        if dry_run:
            print(f"  [{i+1}/{len(rows)}] DRY  {hash_[:12]}.{ext} — {size:,} bytes")
            ok += 1
            continue

        try:
            with conn.cursor() as cur:
                cur.execute(
                    f"UPDATE {table} SET file_size_bytes = %s WHERE hash = %s AND deviceid = %s",
                    (size, hash_, deviceid),
                )
            conn.commit()
            print(f"  [{i+1}/{len(rows)}] OK   {hash_[:12]}.{ext} — {size:,} bytes")
            ok += 1
        except Exception as e:
            print(f"  [{i+1}/{len(rows)}] FAIL {hash_[:12]}.{ext} — {e}")
            conn.rollback()
            fail += 1

    print(f"{table}: ok={ok}  skip={skip}  fail={fail}\n")


def main():
    parser = argparse.ArgumentParser(description="Backfill file_size_bytes for images and videos")
    parser.add_argument("--limit", type=int, default=100000, help="Max rows per table")
    parser.add_argument("--dry-run", action="store_true", help="Print sizes without writing")
    args = parser.parse_args()

    conn = psycopg2.connect(DB_URL)
    backfill_table(conn, "images", IMAGES_DIR, args.limit, args.dry_run)
    backfill_table(conn, "videos", VIDEOS_DIR, args.limit, args.dry_run)
    conn.close()


if __name__ == "__main__":
    main()
