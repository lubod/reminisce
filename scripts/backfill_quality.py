#!/usr/bin/env python3
"""
Backfill quality scores for existing images.
Reads images from DB, calls /quality AI endpoint, writes results back.

Usage:
    python3 scripts/backfill_quality.py [--limit N] [--dry-run]
"""

import argparse
import base64
import json
import os
import sys
import time
import urllib.request
import urllib.error
import psycopg2

DB_URL = os.environ.get("DATABASE_URL", "postgresql://postgres:postgres@localhost:5432/reminisce_db")
AI_URL = os.environ.get("AI_URL", "http://localhost:8081")
IMAGES_DIR = os.environ.get("IMAGES_DIR", "./uploaded_images")


def get_pending(conn, limit):
    with conn.cursor() as cur:
        cur.execute("""
            SELECT hash, ext, deviceid
            FROM images
            WHERE verification_status = 1
              AND embedding IS NOT NULL
              AND quality_score_generated_at IS NULL
            ORDER BY created_at DESC
            LIMIT %s
        """, (limit,))
        return cur.fetchall()


def call_quality(image_data: bytes):
    b64 = base64.b64encode(image_data).decode()
    body = json.dumps({"image": b64}).encode()
    req = urllib.request.Request(
        f"{AI_URL}/quality",
        data=body,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.loads(r.read())


def find_image_path(images_dir, hash_, ext):
    # Subdirectory layout: first 2 chars of hash
    subdir = os.path.join(images_dir, hash_[:2])
    return os.path.join(subdir, f"{hash_}.{ext}")


def mark_permanent_failure(conn, hash_, deviceid):
    with conn.cursor() as cur:
        cur.execute(
            "UPDATE images SET quality_score_generated_at = NOW() WHERE hash = %s AND deviceid = %s",
            (hash_, deviceid),
        )
    conn.commit()


def store_result(conn, hash_, deviceid, result, file_size):
    with conn.cursor() as cur:
        cur.execute("""
            UPDATE images
            SET aesthetic_score = %s,
                sharpness_score = %s,
                width = %s,
                height = %s,
                file_size_bytes = %s,
                quality_score_generated_at = NOW()
            WHERE hash = %s AND deviceid = %s
        """, (
            result["aesthetic_score"],
            result["sharpness_score"],
            result["width"],
            result["height"],
            file_size,
            hash_,
            deviceid,
        ))
    conn.commit()


def main():
    parser = argparse.ArgumentParser(description="Backfill image quality scores")
    parser.add_argument("--limit", type=int, default=10000, help="Max images to process")
    parser.add_argument("--dry-run", action="store_true", help="Query only, don't write results")
    args = parser.parse_args()

    conn = psycopg2.connect(DB_URL)
    rows = get_pending(conn, args.limit)

    print(f"Found {len(rows)} images without quality scores")
    if args.dry_run:
        for hash_, ext, deviceid in rows[:10]:
            print(f"  {hash_}.{ext}  ({deviceid})")
        return

    ok = fail = skip = 0
    for i, (hash_, ext, deviceid) in enumerate(rows):
        path = find_image_path(IMAGES_DIR, hash_, ext)
        if not os.path.exists(path):
            print(f"[{i+1}/{len(rows)}] SKIP {hash_}.{ext} — file not found")
            mark_permanent_failure(conn, hash_, deviceid)
            skip += 1
            continue

        file_size = os.path.getsize(path)
        try:
            with open(path, "rb") as f:
                image_data = f.read()
            result = call_quality(image_data)
            store_result(conn, hash_, deviceid, result, file_size)
            print(f"[{i+1}/{len(rows)}] OK  {hash_[:12]}  aesthetic={result['aesthetic_score']:.1f}  sharpness={result['sharpness_score']:.0f}  {result['width']}x{result['height']}")
            ok += 1
        except urllib.error.HTTPError as e:
            if e.code == 400:
                print(f"[{i+1}/{len(rows)}] SKIP {hash_[:12]} — 400 permanent failure")
                mark_permanent_failure(conn, hash_, deviceid)
                skip += 1
            else:
                print(f"[{i+1}/{len(rows)}] FAIL {hash_[:12]} — HTTP {e.code}: {e.read().decode()[:80]}")
                fail += 1
        except Exception as e:
            print(f"[{i+1}/{len(rows)}] FAIL {hash_[:12]} — {e}")
            fail += 1

        # Small delay to avoid hammering the GPU
        time.sleep(0.1)

    conn.close()
    print(f"\nDone. ok={ok}  skip={skip}  fail={fail}")


if __name__ == "__main__":
    main()
