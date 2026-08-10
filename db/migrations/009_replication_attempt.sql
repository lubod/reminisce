-- Track the last replication attempt time per media file so re-queueing and
-- batch selection can back off from files that repeatedly fail (missing on disk,
-- node outage), preventing retry storms and head-of-line blocking of the queue.
ALTER TABLE images ADD COLUMN IF NOT EXISTS p2p_last_attempt_at TIMESTAMPTZ;
ALTER TABLE videos ADD COLUMN IF NOT EXISTS p2p_last_attempt_at TIMESTAMPTZ;

-- The existing partial indexes (idx_images_need_sync / idx_videos_need_sync on
-- created_at WHERE p2p_synced_at IS NULL) remain the access path; the extra
-- p2p_last_attempt_at backoff predicate is evaluated as a residual filter.
