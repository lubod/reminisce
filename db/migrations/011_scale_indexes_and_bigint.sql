-- Migration 011: Scale indexes, BIGINT encrypted/file sizes, audit attempt tracking, and refresh tokens.

-- Promote file_size_bytes and p2p_encrypted_size to BIGINT to support >2 GiB media
-- without truncation or overflow.
ALTER TABLE images
    ALTER COLUMN file_size_bytes TYPE BIGINT,
    ALTER COLUMN p2p_encrypted_size TYPE BIGINT;

ALTER TABLE videos
    ALTER COLUMN p2p_encrypted_size TYPE BIGINT;

-- Track the last audit attempt per shard row so unreachable/unrepairable shards
-- rotate fairly instead of starving the audit queue.
ALTER TABLE p2p_shards
    ADD COLUMN IF NOT EXISTS last_audit_attempt_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_p2p_shards_audit_order
    ON p2p_shards ((COALESCE(last_audit_attempt_at, last_checked_at)) ASC NULLS FIRST, id ASC);

-- Scale indexes for person face lookups and video full-text search.
CREATE INDEX IF NOT EXISTS idx_faces_user_person
    ON faces(user_id, person_id);

CREATE INDEX IF NOT EXISTS idx_videos_description_fts
    ON videos USING GIN (to_tsvector('english', COALESCE(description, '') || ' ' || COALESCE(name, '')));

-- Revocable refresh tokens so mobile/web clients do not persist plaintext passwords.
CREATE TABLE IF NOT EXISTS refresh_tokens (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash VARCHAR(64) NOT NULL UNIQUE,
    device_id VARCHAR(255),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_used_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_refresh_tokens_user_id ON refresh_tokens(user_id);
CREATE INDEX IF NOT EXISTS idx_refresh_tokens_expires_at ON refresh_tokens(expires_at);
