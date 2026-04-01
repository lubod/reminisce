-- Migration 004: Multi-tenancy — change PK of images/videos to (user_id, hash),
-- add media_sources table to track per-device upload history.
-- All steps are idempotent: safe to run on both old and fresh-install databases.

-- ── Step 1: Create media_sources ──────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS media_sources (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    hash VARCHAR(255) NOT NULL,
    media_type VARCHAR(10) NOT NULL CHECK (media_type IN ('image', 'video')),
    device_id VARCHAR(255) NOT NULL,
    uploaded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (user_id, hash, device_id, media_type)
);
CREATE INDEX IF NOT EXISTS idx_media_sources_user_hash ON media_sources(user_id, hash);
CREATE INDEX IF NOT EXISTS idx_media_sources_device ON media_sources(device_id);

-- ── Step 2: Add new user-id columns to dependent tables (idempotent) ──────────
ALTER TABLE image_labels ADD COLUMN IF NOT EXISTS image_user_id UUID;
ALTER TABLE video_labels ADD COLUMN IF NOT EXISTS video_user_id UUID;
ALTER TABLE faces ADD COLUMN IF NOT EXISTS image_user_id UUID;

-- ── Step 3: Populate new columns from parent tables (only if old deviceid cols exist) ──

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'image_labels' AND column_name = 'image_deviceid') THEN
        UPDATE image_labels il
        SET image_user_id = i.user_id
        FROM images i
        WHERE il.image_hash = i.hash AND il.image_deviceid = i.deviceid
          AND il.image_user_id IS NULL;
    END IF;
END $$;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'video_labels' AND column_name = 'video_deviceid') THEN
        UPDATE video_labels vl
        SET video_user_id = v.user_id
        FROM videos v
        WHERE vl.video_hash = v.hash AND vl.video_deviceid = v.deviceid
          AND vl.video_user_id IS NULL;
    END IF;
END $$;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'faces' AND column_name = 'image_deviceid') THEN
        UPDATE faces f
        SET image_user_id = i.user_id
        FROM images i
        WHERE f.image_hash = i.hash AND f.image_deviceid = i.deviceid
          AND f.image_user_id IS NULL;
    END IF;
END $$;

-- ── Step 4: Populate media_sources from existing data ─────────────────────────
INSERT INTO media_sources (user_id, hash, media_type, device_id, uploaded_at)
SELECT user_id, hash, 'image', deviceid, added_at
FROM images
WHERE user_id IS NOT NULL AND deviceid IS NOT NULL
ON CONFLICT DO NOTHING;

INSERT INTO media_sources (user_id, hash, media_type, device_id, uploaded_at)
SELECT user_id, hash, 'video', deviceid, added_at
FROM videos
WHERE user_id IS NOT NULL AND deviceid IS NOT NULL
ON CONFLICT DO NOTHING;

-- ── Step 5: Prune orphaned rows (no user_id) ──────────────────────────────────
DELETE FROM image_labels WHERE image_user_id IS NULL;
DELETE FROM video_labels WHERE video_user_id IS NULL;
DELETE FROM faces WHERE image_user_id IS NULL;
DELETE FROM images WHERE user_id IS NULL;
DELETE FROM videos WHERE user_id IS NULL;

-- ── Step 6: Drop old FK constraints from dependent tables ─────────────────────
ALTER TABLE faces DROP CONSTRAINT IF EXISTS faces_image_deviceid_image_hash_fkey;
ALTER TABLE image_labels DROP CONSTRAINT IF EXISTS image_labels_image_deviceid_image_hash_fkey;
ALTER TABLE video_labels DROP CONSTRAINT IF EXISTS video_labels_video_deviceid_video_hash_fkey;

-- Drop old indices (will be recreated with new columns in Step 11)
DROP INDEX IF EXISTS idx_faces_image;
DROP INDEX IF EXISTS idx_image_labels_image;
DROP INDEX IF EXISTS idx_video_labels_video;

-- ── Step 7: Drop old PKs (only if still on deviceid columns) ─────────────────
DO $$
BEGIN
    -- images PK
    IF EXISTS (
        SELECT 1 FROM pg_index i
        JOIN pg_class c ON c.oid = i.indrelid
        JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = ANY(i.indkey)
        WHERE c.relname = 'images' AND i.indisprimary AND a.attname = 'deviceid'
    ) THEN
        ALTER TABLE images DROP CONSTRAINT images_pkey;
    END IF;

    -- videos PK
    IF EXISTS (
        SELECT 1 FROM pg_index i
        JOIN pg_class c ON c.oid = i.indrelid
        JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = ANY(i.indkey)
        WHERE c.relname = 'videos' AND i.indisprimary AND a.attname = 'deviceid'
    ) THEN
        ALTER TABLE videos DROP CONSTRAINT videos_pkey;
    END IF;

    -- image_labels PK
    IF EXISTS (
        SELECT 1 FROM pg_index i
        JOIN pg_class c ON c.oid = i.indrelid
        JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = ANY(i.indkey)
        WHERE c.relname = 'image_labels' AND i.indisprimary AND a.attname = 'image_deviceid'
    ) THEN
        ALTER TABLE image_labels DROP CONSTRAINT image_labels_pkey;
    END IF;

    -- video_labels PK
    IF EXISTS (
        SELECT 1 FROM pg_index i
        JOIN pg_class c ON c.oid = i.indrelid
        JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = ANY(i.indkey)
        WHERE c.relname = 'video_labels' AND i.indisprimary AND a.attname = 'video_deviceid'
    ) THEN
        ALTER TABLE video_labels DROP CONSTRAINT video_labels_pkey;
    END IF;
END $$;

-- ── Step 8: Make user_id NOT NULL on images/videos ────────────────────────────
ALTER TABLE images ALTER COLUMN user_id SET NOT NULL;
ALTER TABLE videos ALTER COLUMN user_id SET NOT NULL;

-- ── Step 9: Add new PKs (only if not already user_id-based) ──────────────────
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        JOIN pg_class c ON c.oid = i.indrelid
        JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = ANY(i.indkey)
        WHERE c.relname = 'images' AND i.indisprimary AND a.attname = 'user_id'
    ) THEN
        ALTER TABLE images ADD PRIMARY KEY (user_id, hash);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        JOIN pg_class c ON c.oid = i.indrelid
        JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = ANY(i.indkey)
        WHERE c.relname = 'videos' AND i.indisprimary AND a.attname = 'user_id'
    ) THEN
        ALTER TABLE videos ADD PRIMARY KEY (user_id, hash);
    END IF;
END $$;

-- ── Step 10: Make new user_id columns NOT NULL in dependent tables ─────────────
ALTER TABLE image_labels ALTER COLUMN image_user_id SET NOT NULL;
ALTER TABLE video_labels ALTER COLUMN video_user_id SET NOT NULL;
ALTER TABLE faces ALTER COLUMN image_user_id SET NOT NULL;

-- ── Step 11: Add new PKs to label tables ──────────────────────────────────────
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        JOIN pg_class c ON c.oid = i.indrelid
        JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = ANY(i.indkey)
        WHERE c.relname = 'image_labels' AND i.indisprimary AND a.attname = 'image_user_id'
    ) THEN
        ALTER TABLE image_labels ADD PRIMARY KEY (image_hash, image_user_id, label_id);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        JOIN pg_class c ON c.oid = i.indrelid
        JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = ANY(i.indkey)
        WHERE c.relname = 'video_labels' AND i.indisprimary AND a.attname = 'video_user_id'
    ) THEN
        ALTER TABLE video_labels ADD PRIMARY KEY (video_hash, video_user_id, label_id);
    END IF;
END $$;

-- ── Step 12: Add new FKs ──────────────────────────────────────────────────────
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'image_labels_image_user_id_image_hash_fkey') THEN
        ALTER TABLE image_labels ADD CONSTRAINT image_labels_image_user_id_image_hash_fkey
            FOREIGN KEY (image_user_id, image_hash) REFERENCES images(user_id, hash) ON DELETE CASCADE;
    END IF;

    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'video_labels_video_user_id_video_hash_fkey') THEN
        ALTER TABLE video_labels ADD CONSTRAINT video_labels_video_user_id_video_hash_fkey
            FOREIGN KEY (video_user_id, video_hash) REFERENCES videos(user_id, hash) ON DELETE CASCADE;
    END IF;

    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'faces_image_user_id_image_hash_fkey') THEN
        ALTER TABLE faces ADD CONSTRAINT faces_image_user_id_image_hash_fkey
            FOREIGN KEY (image_user_id, image_hash) REFERENCES images(user_id, hash) ON DELETE CASCADE;
    END IF;
END $$;

-- ── Step 13: Drop old deviceid columns from dependent tables ──────────────────
ALTER TABLE image_labels DROP COLUMN IF EXISTS image_deviceid;
ALTER TABLE video_labels DROP COLUMN IF EXISTS video_deviceid;
ALTER TABLE faces DROP COLUMN IF EXISTS image_deviceid;

-- ── Step 14: Rebuild indices with new columns ─────────────────────────────────
CREATE INDEX IF NOT EXISTS idx_faces_image ON faces(image_user_id, image_hash);
CREATE INDEX IF NOT EXISTS idx_image_labels_image ON image_labels(image_user_id, image_hash);
CREATE INDEX IF NOT EXISTS idx_video_labels_video ON video_labels(video_user_id, video_hash);
