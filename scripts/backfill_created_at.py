#!/usr/bin/env python3
"""
Fix created_at for images whose created_at was set to upload time.

Tries date sources in priority order:
  1. EXIF DateTimeOriginal  (already done for 279 images — skipped if created_at already fixed)
  2. EXIF DateTime / DateTimeDigitized  (fallback EXIF fields)
  3. Filename patterns:
       IMG-YYYYMMDD-WA*          WhatsApp (date only → noon UTC)
       IMG_YYYYMMDD_HHMMSS*      Camera
       VID_YYYYMMDD_HHMMSS*      Camera video
       YYYY-MM-DD*               ISO date prefix
       YYYYMMDD_HHMMSS*          Generic
       <13-digit unix ms>.*      Unix millisecond timestamp
  4. File mtime on disk  (opt-in via --use-mtime, lowest confidence)

Usage:
    python3 scripts/backfill_created_at.py [--limit N] [--dry-run] [--use-mtime]

Dependencies:
    pip install psycopg2-binary
"""

import argparse
import os
import re
import time
from datetime import datetime, timezone, timedelta

import psycopg2

DB_URL     = os.environ.get("DATABASE_URL", "postgresql://postgres:postgres@localhost:5432/reminisce_db")
IMAGES_DIR = os.environ.get("IMAGES_DIR",  "./uploaded_images")


# ---------------------------------------------------------------------------
# Date parsing helpers
# ---------------------------------------------------------------------------

def parse_exif_dt(s: str) -> datetime | None:
    if not s:
        return None
    for fmt in ("%Y-%m-%d %H:%M:%S", "%Y:%m:%d %H:%M:%S"):
        try:
            return datetime.strptime(s, fmt).replace(tzinfo=timezone.utc)
        except ValueError:
            continue
    return None


# Compiled filename patterns, tried in order
_FILENAME_PATTERNS = [
    # WhatsApp: IMG-20250731-WA0000.jpg  (date only → noon UTC)
    (re.compile(r"IMG-(\d{4})(\d{2})(\d{2})-WA", re.IGNORECASE),
     lambda m: datetime(int(m[1]), int(m[2]), int(m[3]), 12, 0, 0, tzinfo=timezone.utc),
     "WhatsApp filename"),

    # Camera: IMG_20250731_141530 or VID_20250731_141530
    (re.compile(r"(?:IMG|VID)_(\d{4})(\d{2})(\d{2})_(\d{2})(\d{2})(\d{2})", re.IGNORECASE),
     lambda m: datetime(int(m[1]), int(m[2]), int(m[3]),
                        int(m[4]), int(m[5]), int(m[6]), tzinfo=timezone.utc),
     "Camera filename"),

    # Generic YYYYMMDD_HHMMSS
    (re.compile(r"(\d{4})(\d{2})(\d{2})_(\d{2})(\d{2})(\d{2})"),
     lambda m: datetime(int(m[1]), int(m[2]), int(m[3]),
                        int(m[4]), int(m[5]), int(m[6]), tzinfo=timezone.utc),
     "YYYYMMDD_HHMMSS filename"),

    # ISO date prefix: 2025-07-31 or 2025-07-31T14:15
    (re.compile(r"(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2}):(\d{2})(?::(\d{2}))?)?"),
     lambda m: datetime(int(m[1]), int(m[2]), int(m[3]),
                        int(m[4] or 12), int(m[5] or 0), int(m[6] or 0),
                        tzinfo=timezone.utc),
     "ISO date filename"),

    # Unix milliseconds: 1754033434572.jpg  (13 digits)
    (re.compile(r"(?:^|/)(\d{13})\."),
     lambda m: datetime.fromtimestamp(int(m[1]) / 1000, tz=timezone.utc),
     "Unix ms filename"),
]


def date_from_filename(name: str) -> tuple[datetime, str] | None:
    """Extract date from filename using patterns above. Returns (dt, source) or None."""
    basename = os.path.basename(name)
    for pattern, builder, source in _FILENAME_PATTERNS:
        m = pattern.search(basename) or pattern.search(name)
        if m:
            try:
                dt = builder(m)
                # Sanity: date must be reasonable (2000–2030)
                if 2000 <= dt.year <= 2030:
                    return dt, source
            except ValueError:
                continue
    return None


def date_from_mtime(images_dir: str, hash_: str, ext: str) -> tuple[datetime, str] | None:
    path = os.path.join(images_dir, hash_[:2], f"{hash_}.{ext}")
    try:
        mtime = os.path.getmtime(path)
        return datetime.fromtimestamp(mtime, tz=timezone.utc), "file mtime"
    except OSError:
        return None


# ---------------------------------------------------------------------------
# DB helpers
# ---------------------------------------------------------------------------

def get_pending(conn, limit: int) -> list:
    """All images whose created_at still looks like upload time."""
    with conn.cursor() as cur:
        cur.execute(
            """
            SELECT hash, ext, deviceid, created_at, added_at,
                   exif::jsonb->>'DateTimeOriginal'  as dto,
                   exif::jsonb->>'DateTime'          as dt,
                   exif::jsonb->>'DateTimeDigitized' as dtd,
                   name
            FROM images
            WHERE ABS(EXTRACT(EPOCH FROM (created_at - added_at))) < 60
              AND deleted_at IS NULL
            ORDER BY added_at DESC
            LIMIT %s
            """,
            (limit,),
        )
        return cur.fetchall()


def apply_fix(conn, hash_: str, deviceid: str, dt: datetime) -> bool:
    with conn.cursor() as cur:
        cur.execute(
            """
            UPDATE images SET created_at = %s
            WHERE hash = %s AND deviceid = %s
              AND ABS(EXTRACT(EPOCH FROM (created_at - added_at))) < 60
            """,
            (dt, hash_, deviceid),
        )
        updated = cur.rowcount
    conn.commit()
    return updated > 0


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    parser = argparse.ArgumentParser(description="Fix image created_at from all available date sources")
    parser.add_argument("--limit", type=int, default=10000)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--use-mtime", action="store_true",
                        help="Fall back to file mtime for images with no other date source")
    args = parser.parse_args()

    conn = psycopg2.connect(DB_URL)
    rows = get_pending(conn, args.limit)
    print(f"Found {len(rows)} images with created_at ≈ upload time")

    if args.dry_run:
        source_counts = {}
        no_source = 0
        for hash_, ext, deviceid, created_at, added_at, dto, dt_exif, dtd, name in rows[:200]:
            dt = source = None
            for val, lbl in [(dto, "EXIF DateTimeOriginal"),
                             (dt_exif, "EXIF DateTime"),
                             (dtd, "EXIF DateTimeDigitized")]:
                dt = parse_exif_dt(val)
                if dt:
                    source = lbl
                    break
            if not dt:
                r = date_from_filename(name)
                if r:
                    dt, source = r
            if not dt and args.use_mtime:
                r = date_from_mtime(IMAGES_DIR, hash_, ext)
                if r:
                    dt, source = r
            if dt:
                source_counts[source] = source_counts.get(source, 0) + 1
                print(f"  {hash_[:12]}  {created_at.date()} → {dt.date()}  [{source}]")
            else:
                no_source += 1

        print(f"\nSummary (first {min(200, len(rows))} rows):")
        for src, cnt in sorted(source_counts.items(), key=lambda x: -x[1]):
            print(f"  {cnt:4d}  {src}")
        print(f"  {no_source:4d}  no source found")
        return

    ok = skip = fail = 0
    source_counts = {}

    for i, (hash_, ext, deviceid, created_at, added_at, dto, dt_exif, dtd, name) in enumerate(rows):
        dt = source = None

        # Priority 1: EXIF datetime fields
        for val, lbl in [(dto,     "EXIF DateTimeOriginal"),
                         (dt_exif, "EXIF DateTime"),
                         (dtd,     "EXIF DateTimeDigitized")]:
            dt = parse_exif_dt(val)
            if dt:
                source = lbl
                break

        # Priority 2: filename
        if not dt:
            r = date_from_filename(name)
            if r:
                dt, source = r

        # Priority 3: file mtime (opt-in)
        if not dt and args.use_mtime:
            r = date_from_mtime(IMAGES_DIR, hash_, ext)
            if r:
                dt, source = r

        if not dt:
            print(f"[{i+1}/{len(rows)}] SKIP {hash_[:12]} — no date source  name={os.path.basename(name)}")
            skip += 1
            continue

        try:
            if apply_fix(conn, hash_, deviceid, dt):
                source_counts[source] = source_counts.get(source, 0) + 1
                print(f"[{i+1}/{len(rows)}] OK  {hash_[:12]}  {created_at.date()} → {dt.date()}  [{source}]")
                ok += 1
            else:
                print(f"[{i+1}/{len(rows)}] SKIP {hash_[:12]} — already fixed")
                skip += 1
        except Exception as e:
            print(f"[{i+1}/{len(rows)}] FAIL {hash_[:12]} — {e}")
            conn.rollback()
            fail += 1

        time.sleep(0.005)

    conn.close()
    print(f"\nDone. ok={ok}  skip={skip}  fail={fail}")
    print("Sources used:")
    for src, cnt in sorted(source_counts.items(), key=lambda x: -x[1]):
        print(f"  {cnt:4d}  {src}")


if __name__ == "__main__":
    main()
