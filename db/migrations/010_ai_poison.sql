-- AI description poison hardening.
-- description_failed_attempts: counts consecutive non-permanent (e.g. transport)
--   description failures per row, so a deterministic crasher is parked as
--   [skipped] after a cap instead of retrying forever.
-- description_started_at: timestamp of the [__processing__] marker write, so the
--   un-park sweep can gate on when the row was actually started (not ingested).
ALTER TABLE images ADD COLUMN IF NOT EXISTS description_failed_attempts INT NOT NULL DEFAULT 0;
ALTER TABLE videos ADD COLUMN IF NOT EXISTS description_failed_attempts INT NOT NULL DEFAULT 0;
ALTER TABLE images ADD COLUMN IF NOT EXISTS description_started_at TIMESTAMPTZ;
ALTER TABLE videos ADD COLUMN IF NOT EXISTS description_started_at TIMESTAMPTZ;
