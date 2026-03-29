ALTER TABLE images ADD COLUMN IF NOT EXISTS duplicates_checked_at TIMESTAMPTZ;

CREATE TABLE IF NOT EXISTS image_duplicate_pairs (
    hash_a       VARCHAR(255) NOT NULL,
    hash_b       VARCHAR(255) NOT NULL,
    similarity   REAL NOT NULL,
    user_id      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    computed_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (hash_a, hash_b, user_id)
);

CREATE INDEX IF NOT EXISTS idx_dup_pairs_user_sim ON image_duplicate_pairs(user_id, similarity DESC);

CREATE INDEX IF NOT EXISTS idx_images_dup_pending ON images(duplicates_checked_at)
    WHERE duplicates_checked_at IS NULL AND embedding IS NOT NULL AND deleted_at IS NULL;
