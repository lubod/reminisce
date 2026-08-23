-- Idempotent migrations from the 2026-08 deep code review.
-- Apply with: psql "$DATABASE_URL" -f scripts/migrations/2026-08-23_review_indexes.sql
-- All statements are safe to re-run.

-- 1. Orientation worker scan: full-table seq scan every cycle while a backlog
--    exists. Partial index matches the exact WHERE clause the worker uses.
CREATE INDEX IF NOT EXISTS images_orientation_backlog_idx
    ON images (orientation_detected_at)
    WHERE exif IS NULL
      AND orientation IS NULL
      AND orientation_detected_at IS NULL
      AND verification_status = 1
      AND deleted_at IS NULL;

-- 2. Geocoding hot path: admin_boundaries.name ILIKE %q% seq-scans on every
--    place search. pg_trgm GIN makes substring search index-assisted.
CREATE EXTENSION IF NOT EXISTS pg_trgm;
CREATE INDEX IF NOT EXISTS admin_boundaries_name_trgm_idx
    ON admin_boundaries USING gin (name gin_trgm_ops);

-- 2026-08-23b: orientation backlog widened — AI detection now also scans
-- EXIF-bearing photos whose files lack an Orientation tag (the actual cause
-- of wrongly-oriented photos). Old index matched the retired predicate.
DROP INDEX IF EXISTS images_orientation_backlog_idx;
CREATE INDEX IF NOT EXISTS images_orientation_backlog_idx
    ON images (orientation_detected_at)
    WHERE orientation IS NULL
      AND orientation_detected_at IS NULL
      AND verification_status = 1
      AND deleted_at IS NULL;
