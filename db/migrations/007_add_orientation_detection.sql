-- Track when AI-fallback orientation detection has been attempted for an image.
-- EXIF-first detection (ingest.rs) never runs for files without EXIF, so images that
-- carry no EXIF orientation stay NULL and are picked up by the AI worker to be
-- detected via the image-classifier fallback. orientation_detected_at is set on every
-- attempted detection (success OR permanent failure) so the worker does not retry
-- forever; orientation stays NULL on failure.
ALTER TABLE images ADD COLUMN IF NOT EXISTS orientation_detected_at TIMESTAMPTZ;
