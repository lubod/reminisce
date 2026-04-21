-- Adds segmented large-file sharding columns to images and videos tables.
-- Applied automatically on fresh installs via init.sql (idempotent ADD COLUMN IF NOT EXISTS).
-- Run manually on existing deployments: psql -U postgres reminisce_db -f this file

ALTER TABLE images
    ADD COLUMN IF NOT EXISTS p2p_segment_count INTEGER NOT NULL DEFAULT 1,
    ADD COLUMN IF NOT EXISTS p2p_segment_enc_sizes BIGINT[] DEFAULT NULL;

ALTER TABLE videos
    ADD COLUMN IF NOT EXISTS p2p_segment_count INTEGER NOT NULL DEFAULT 1,
    ADD COLUMN IF NOT EXISTS p2p_segment_enc_sizes BIGINT[] DEFAULT NULL;
