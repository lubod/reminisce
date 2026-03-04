#!/usr/bin/env python3
"""
Backfill EXIF for images that have none stored in the DB.
For each image with exif IS NULL:
  - Extract EXIF from the file on disk using exifread
  - Store the EXIF JSON in the DB
  - If DateTimeOriginal found and created_at looks like upload time, fix created_at
  - If GPS found, set location (PostGIS) and reverse-geocode place

Usage:
    python3 scripts/backfill_exif.py [--limit N] [--dry-run]

Dependencies:
    pip install psycopg2-binary exifread
"""

import argparse
import json
import os
import sys
import time
from datetime import datetime, timezone

import psycopg2
import psycopg2.extras

try:
    import exifread
except ImportError:
    print("ERROR: exifread not installed. Run: pip install exifread")
    sys.exit(1)

DB_URL      = os.environ.get("DATABASE_URL",  "postgresql://postgres:postgres@localhost:5432/reminisce_db")
GEO_DB_URL  = os.environ.get("GEO_DB_URL",   "postgresql://postgres:postgres@localhost:5435/geotagging_db")
IMAGES_DIR  = os.environ.get("IMAGES_DIR",   "./uploaded_images")

# Tag-name prefix stripping (exifread prefixes: "Image ", "EXIF ", "GPS ", "Interop ", etc.)
_STRIP_PREFIXES = ("Image ", "EXIF ", "GPS ", "Interop ", "Thumbnail ", "MakerNote ")


def clean_tag(tag_name: str) -> str:
    for prefix in _STRIP_PREFIXES:
        if tag_name.startswith(prefix):
            return tag_name[len(prefix):]
    return tag_name


def exif_value_to_str(val) -> str:
    """Convert any exifread tag value to a human-readable string."""
    return str(val)


def extract_exif(path: str) -> dict | None:
    """Return dict of {tag: value_str} or None if no EXIF found."""
    try:
        with open(path, "rb") as f:
            tags = exifread.process_file(f, details=False, stop_tag="EOF")
    except Exception as e:
        return None

    if not tags:
        return None

    result = {}
    for raw_key, val in tags.items():
        key = clean_tag(raw_key)
        result[key] = exif_value_to_str(val)
    return result if result else None


def rational_to_float(rat_str: str) -> float | None:
    """Parse IFDRational string like '52' or '37/1' to float."""
    rat_str = rat_str.strip()
    if "/" in rat_str:
        num, den = rat_str.split("/", 1)
        den = den.strip()
        if den == "0":
            return None
        return float(num) / float(den)
    try:
        return float(rat_str)
    except ValueError:
        return None


def parse_gps_tag(tag_val_str: str, ref: str) -> float | None:
    """
    exifread represents GPS as e.g. '[52, 31, 1199/100]'
    Convert to decimal degrees, applying N/S/E/W sign.
    """
    # Strip brackets
    tag_val_str = tag_val_str.strip("[]")
    parts = [p.strip() for p in tag_val_str.split(",")]
    if len(parts) < 3:
        return None

    vals = [rational_to_float(p) for p in parts]
    if any(v is None for v in vals):
        return None

    degrees, minutes, seconds = vals[0], vals[1], vals[2]
    decimal = degrees + minutes / 60.0 + seconds / 3600.0

    if ref.strip().upper() in ("S", "W"):
        decimal = -decimal
    return decimal


def extract_gps(exif_dict: dict) -> tuple[float, float] | None:
    """Return (lat, lon) decimal or None."""
    lat_str  = exif_dict.get("GPSLatitude")
    lon_str  = exif_dict.get("GPSLongitude")
    lat_ref  = exif_dict.get("GPSLatitudeRef")
    lon_ref  = exif_dict.get("GPSLongitudeRef")

    if not all([lat_str, lon_str, lat_ref, lon_ref]):
        return None

    lat = parse_gps_tag(lat_str, lat_ref)
    lon = parse_gps_tag(lon_str, lon_ref)
    if lat is None or lon is None:
        return None
    if not (-90 <= lat <= 90 and -180 <= lon <= 180):
        return None
    return lat, lon


def reverse_geocode(geo_conn, lat: float, lon: float) -> str | None:
    """Query local admin_boundaries PostGIS table."""
    try:
        with geo_conn.cursor() as cur:
            cur.execute(
                """
                SELECT name FROM admin_boundaries
                WHERE ST_Contains(geometry, ST_SetSRID(ST_MakePoint(%s, %s), 4326))
                ORDER BY admin_level DESC
                """,
                (lon, lat),
            )
            rows = cur.fetchall()
        if not rows:
            return None
        seen = []
        for (name,) in rows:
            if name not in seen:
                seen.append(name)
        return ", ".join(seen)
    except Exception as e:
        print(f"    Geocoding error: {e}")
        return None


def parse_dto(dto_str: str) -> datetime | None:
    """Parse DateTimeOriginal string '2025-07-29 14:05:55' to UTC datetime."""
    for fmt in ("%Y-%m-%d %H:%M:%S", "%Y:%m:%d %H:%M:%S"):
        try:
            return datetime.strptime(dto_str, fmt).replace(tzinfo=timezone.utc)
        except ValueError:
            continue
    return None


def get_pending(conn, limit: int) -> list:
    with conn.cursor() as cur:
        cur.execute(
            """
            SELECT hash, ext, deviceid, added_at
            FROM images
            WHERE exif IS NULL
              AND deleted_at IS NULL
            ORDER BY created_at DESC
            LIMIT %s
            """,
            (limit,),
        )
        return cur.fetchall()


def image_path(images_dir: str, hash_: str, ext: str) -> str:
    return os.path.join(images_dir, hash_[:2], f"{hash_}.{ext}")


def store(conn, geo_conn, hash_: str, deviceid: str, added_at, exif_dict: dict, dry_run: bool) -> dict:
    """Write EXIF, optionally fix created_at and location. Returns summary dict."""
    result = {"exif": True, "created_at_fixed": False, "location_set": False, "place": None}

    exif_json = json.dumps(exif_dict)

    # GPS
    gps = extract_gps(exif_dict)
    lat, lon = gps if gps else (None, None)

    # DateTimeOriginal
    dto_str = exif_dict.get("DateTimeOriginal")
    dto = parse_dto(dto_str) if dto_str else None

    if dry_run:
        result["dto"] = dto_str
        result["gps"] = gps
        return result

    with conn.cursor() as cur:
        # 1. Store EXIF (only if still NULL — avoid races)
        cur.execute(
            "UPDATE images SET exif = %s WHERE hash = %s AND deviceid = %s AND exif IS NULL",
            (exif_json, hash_, deviceid),
        )

        # 2. Fix created_at if it looks like upload time (within 60s of added_at)
        if dto:
            cur.execute(
                """
                UPDATE images SET created_at = %s
                WHERE hash = %s AND deviceid = %s
                  AND ABS(EXTRACT(EPOCH FROM (created_at - %s))) < 60
                """,
                (dto, hash_, deviceid, added_at),
            )
            if cur.rowcount > 0:
                result["created_at_fixed"] = True

        # 3. Set GPS location
        if lat is not None:
            cur.execute(
                """
                UPDATE images
                SET location = ST_SetSRID(ST_MakePoint(%s, %s), 4326)
                WHERE hash = %s AND deviceid = %s AND location IS NULL
                """,
                (lon, lat, hash_, deviceid),
            )
            if cur.rowcount > 0:
                result["location_set"] = True
                place = reverse_geocode(geo_conn, lat, lon)
                if place:
                    cur.execute(
                        "UPDATE images SET place = %s WHERE hash = %s AND deviceid = %s AND place IS NULL",
                        (place, hash_, deviceid),
                    )
                    result["place"] = place

    conn.commit()
    return result


def main():
    parser = argparse.ArgumentParser(description="Backfill EXIF for images missing it in the DB")
    parser.add_argument("--limit", type=int, default=10000, help="Max images to process")
    parser.add_argument("--dry-run", action="store_true", help="Query only, don't write results")
    args = parser.parse_args()

    conn = psycopg2.connect(DB_URL)
    geo_conn = psycopg2.connect(GEO_DB_URL)

    rows = get_pending(conn, args.limit)
    print(f"Found {len(rows)} images with exif IS NULL")

    if args.dry_run:
        for hash_, ext, deviceid, added_at in rows[:10]:
            path = image_path(IMAGES_DIR, hash_, ext)
            exif_dict = extract_exif(path) if os.path.exists(path) else None
            gps = extract_gps(exif_dict) if exif_dict else None
            dto = exif_dict.get("DateTimeOriginal") if exif_dict else None
            print(f"  {hash_[:12]}.{ext}  exif={'yes' if exif_dict else 'none'}  "
                  f"gps={gps}  dto={dto}")
        return

    ok = no_file = no_exif = fail = 0
    dto_fixed = gps_set = 0

    for i, (hash_, ext, deviceid, added_at) in enumerate(rows):
        path = image_path(IMAGES_DIR, hash_, ext)

        if not os.path.exists(path):
            print(f"[{i+1}/{len(rows)}] SKIP {hash_[:12]}.{ext} — file not found")
            no_file += 1
            continue

        exif_dict = extract_exif(path)
        if not exif_dict:
            print(f"[{i+1}/{len(rows)}] NO-EXIF {hash_[:12]}.{ext}")
            no_exif += 1
            continue

        try:
            res = store(conn, geo_conn, hash_, deviceid, added_at, exif_dict, args.dry_run)
            tags = len(exif_dict)
            extra = []
            if res["created_at_fixed"]:
                extra.append(f"date={exif_dict.get('DateTimeOriginal','?')}")
                dto_fixed += 1
            if res["location_set"]:
                extra.append(f"place={res['place'] or 'no-place'}")
                gps_set += 1
            print(f"[{i+1}/{len(rows)}] OK  {hash_[:12]}.{ext}  {tags} tags  {' | '.join(extra)}")
            ok += 1
        except Exception as e:
            print(f"[{i+1}/{len(rows)}] FAIL {hash_[:12]}.{ext} — {e}")
            conn.rollback()
            fail += 1

        time.sleep(0.05)

    conn.close()
    geo_conn.close()
    print(f"\nDone. ok={ok}  no_file={no_file}  no_exif={no_exif}  fail={fail}")
    print(f"      created_at_fixed={dto_fixed}  gps_set={gps_set}")


if __name__ == "__main__":
    main()
