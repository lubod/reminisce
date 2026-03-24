-- Recreate partial indexes to exclude soft-deleted rows.
-- IF NOT EXISTS won't update an index whose WHERE condition changed, so we drop+recreate.

DROP INDEX IF EXISTS idx_images_embedding_status;
CREATE INDEX idx_images_embedding_status ON images(embedding_generated_at)
WHERE embedding_generated_at IS NULL AND deleted_at IS NULL;

DROP INDEX IF EXISTS idx_images_face_detection_pending;
CREATE INDEX idx_images_face_detection_pending ON images(face_detection_completed_at)
WHERE face_detection_completed_at IS NULL AND deleted_at IS NULL;

DROP INDEX IF EXISTS idx_images_quality_pending;
CREATE INDEX idx_images_quality_pending ON images(quality_score_generated_at)
WHERE quality_score_generated_at IS NULL AND deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_images_description_pending ON images(created_at)
WHERE description IS NULL AND deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_images_verification_pending ON images(verification_status, last_verified_at)
WHERE deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_videos_verification_pending ON videos(verification_status, last_verified_at)
WHERE deleted_at IS NULL;
