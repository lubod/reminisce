-- This SQL file will be executed automatically when the PostgreSQL container starts
-- if the database is empty, which is perfect for initializing our schema.

-- Enable PostGIS extension for geospatial support
CREATE EXTENSION IF NOT EXISTS postgis;

-- Enable pgvector extension for vector similarity search (AI-powered semantic search)
CREATE EXTENSION IF NOT EXISTS vector;

-- Users table for authentication (must be created first due to foreign key constraints)
CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    username VARCHAR(255) UNIQUE NOT NULL,
    email VARCHAR(255) UNIQUE NOT NULL,
    password_hash VARCHAR(255) NOT NULL,
    role VARCHAR(50) NOT NULL DEFAULT 'user',
    is_active BOOLEAN DEFAULT TRUE,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    last_login_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_users_username ON users(username);
CREATE INDEX IF NOT EXISTS idx_users_email ON users(email);

-- Create the tables if they don't exist (same as schema.sql)
CREATE TABLE IF NOT EXISTS images (
    deviceid VARCHAR(255) NOT NULL,
    hash VARCHAR(255) NOT NULL,
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    type VARCHAR(255),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    added_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    name VARCHAR(255) NOT NULL,
    ext VARCHAR(10) NOT NULL,
    exif TEXT,
    has_thumbnail BOOLEAN DEFAULT FALSE,
    last_verified_at TIMESTAMPTZ,
    verification_status INTEGER NOT NULL DEFAULT 0 CONSTRAINT chk_images_verification_status CHECK (verification_status IN (-1, 0, 1)), -- 0: not verified/pending, 1: OK/verified, -1: NOK/failed
    location GEOGRAPHY(POINT, 4326), -- WGS84 coordinate system for GPS data
    place TEXT,
    description TEXT, -- AI-generated description of the image
    embedding vector(1152), -- SigLIP image embedding vector (1152-dimensional) for semantic search
    embedding_generated_at TIMESTAMPTZ, -- Timestamp when embedding was generated
    face_detection_completed_at TIMESTAMPTZ, -- Timestamp when face detection was completed (even if 0 faces found)
    deleted_at TIMESTAMPTZ, -- Timestamp for soft deletion
    p2p_synced_at TIMESTAMPTZ, -- Timestamp when this media file was last synced to P2P network. NULL means needs sync.
    p2p_shard_hash VARCHAR(255), -- Root hash or manifest hash of the object in P2P network (Blake3).
    p2p_encryption_key BYTEA, -- 32-byte encryption key used for sharding (needed for re-sharding during rebalance)
    p2p_encrypted_size INTEGER, -- Size of the encrypted blob before erasure coding (needed for reconstruction)
    aesthetic_score REAL, -- AI-computed aesthetic quality score (0–10)
    sharpness_score REAL, -- Laplacian variance sharpness score
    width INTEGER, -- Image width in pixels
    height INTEGER, -- Image height in pixels
    file_size_bytes INTEGER, -- File size in bytes
    quality_score_generated_at TIMESTAMPTZ, -- Timestamp when quality scores were computed
    PRIMARY KEY (deviceid, hash)
);

CREATE INDEX IF NOT EXISTS idx_deviceid_type_created_at ON images(deviceid, type, created_at DESC);
-- Index for soft delete filtering
CREATE INDEX IF NOT EXISTS idx_images_deleted_at ON images(deleted_at) WHERE deleted_at IS NULL;
-- Index for verification status
CREATE INDEX IF NOT EXISTS idx_images_verification_status ON images(verification_status);
-- Composite for verification worker: WHERE deleted_at IS NULL AND (verification_status = 0/1/-1 ...)
CREATE INDEX IF NOT EXISTS idx_images_verification_pending ON images(verification_status, last_verified_at) WHERE deleted_at IS NULL;
-- Spatial index for location-based queries (e.g., find images near a location)
CREATE INDEX IF NOT EXISTS idx_images_location ON images USING GIST(location);
-- Index for user_id queries
CREATE INDEX IF NOT EXISTS idx_images_user_id ON images(user_id);
-- HNSW index for fast approximate nearest neighbor search (semantic image search)
-- m=16: number of connections per layer
-- ef_construction=64: build quality (higher = better but slower indexing)
CREATE INDEX IF NOT EXISTS idx_images_embedding_hnsw ON images
USING hnsw (embedding vector_cosine_ops)
WITH (m = 16, ef_construction = 64);
-- Index to track images without embeddings for backfill worker (excludes soft-deleted)
CREATE INDEX IF NOT EXISTS idx_images_embedding_status ON images(embedding_generated_at)
WHERE embedding_generated_at IS NULL AND deleted_at IS NULL;
-- Index for fast hash lookups (used in get_image, toggle_star, metadata queries)
CREATE INDEX IF NOT EXISTS idx_images_hash ON images(hash);
-- Partial index for thumbnail queries (only index rows with thumbnails)
CREATE INDEX IF NOT EXISTS idx_images_has_thumbnail ON images(has_thumbnail) WHERE has_thumbnail = true;
-- Full-text search index on description and name for fast text-based search
-- Uses GIN index with english text search configuration
-- This enables queries like: WHERE to_tsvector('english', description) @@ plainto_tsquery('english', 'sunset beach')
CREATE INDEX IF NOT EXISTS idx_images_description_fts ON images
USING GIN (to_tsvector('english', COALESCE(description, '') || ' ' || COALESCE(name, '')));
-- Partial index for existence checks (indexes hash, not the text value, to avoid btree row size limits)
CREATE INDEX IF NOT EXISTS idx_images_description_exists ON images(hash) WHERE description IS NOT NULL AND description != '';
-- Index to find images needing AI description (excludes soft-deleted)
CREATE INDEX IF NOT EXISTS idx_images_description_pending ON images(created_at) WHERE description IS NULL AND deleted_at IS NULL;
-- Index to find images that haven't had face detection run yet (excludes soft-deleted)
CREATE INDEX IF NOT EXISTS idx_images_face_detection_pending ON images(face_detection_completed_at) WHERE face_detection_completed_at IS NULL AND deleted_at IS NULL;
-- Index for finding unsynced images efficiently (P2P media replication)
CREATE INDEX IF NOT EXISTS idx_images_need_sync ON images(created_at) WHERE p2p_synced_at IS NULL;
-- Index for looking up P2P synced status
CREATE INDEX IF NOT EXISTS idx_images_p2p_synced ON images(p2p_synced_at);
-- Composite index for main gallery query: WHERE user_id = $N AND deleted_at IS NULL ORDER BY created_at DESC
CREATE INDEX IF NOT EXISTS idx_images_user_created ON images(user_id, created_at DESC) WHERE deleted_at IS NULL;
-- Index to find images that haven't had quality scoring run yet (excludes soft-deleted)
CREATE INDEX IF NOT EXISTS idx_images_quality_pending ON images(quality_score_generated_at) WHERE quality_score_generated_at IS NULL AND deleted_at IS NULL;

-- Idempotent migrations for existing databases
ALTER TABLE images ADD COLUMN IF NOT EXISTS aesthetic_score REAL;
ALTER TABLE images ADD COLUMN IF NOT EXISTS sharpness_score REAL;
ALTER TABLE images ADD COLUMN IF NOT EXISTS width INTEGER;
ALTER TABLE images ADD COLUMN IF NOT EXISTS height INTEGER;
ALTER TABLE images ADD COLUMN IF NOT EXISTS file_size_bytes INTEGER;
ALTER TABLE images ADD COLUMN IF NOT EXISTS quality_score_generated_at TIMESTAMPTZ;

-- Starred images table for per-user image starring (cross-device)
CREATE TABLE IF NOT EXISTS starred_images (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    hash VARCHAR(255) NOT NULL,
    starred_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, hash)
);

CREATE INDEX IF NOT EXISTS idx_starred_images_user_id ON starred_images(user_id);
-- Composite covers the gallery LEFT JOIN: ON t.hash = s.hash AND s.user_id = $N
CREATE INDEX IF NOT EXISTS idx_starred_images_hash_user ON starred_images(hash, user_id);

CREATE TABLE IF NOT EXISTS starred_videos (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    hash VARCHAR(255) NOT NULL,
    starred_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, hash)
);

CREATE INDEX IF NOT EXISTS idx_starred_videos_user_id ON starred_videos(user_id);
-- Composite covers the gallery LEFT JOIN: ON t.hash = s.hash AND s.user_id = $N
CREATE INDEX IF NOT EXISTS idx_starred_videos_hash_user ON starred_videos(hash, user_id);

CREATE TABLE IF NOT EXISTS videos (
    deviceid VARCHAR(255) NOT NULL,
    hash VARCHAR(255) NOT NULL,
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    type VARCHAR(255),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    added_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    name VARCHAR(255) NOT NULL,
    ext VARCHAR(10) NOT NULL,
    metadata TEXT,
    has_thumbnail BOOLEAN DEFAULT FALSE,
    last_verified_at TIMESTAMPTZ,
    verification_status INTEGER NOT NULL DEFAULT 0 CONSTRAINT chk_videos_verification_status CHECK (verification_status IN (-1, 0, 1)), -- 0: not verified/pending, 1: OK/verified, -1: NOK/failed
    description TEXT, -- AI-generated description of the video
    deleted_at TIMESTAMPTZ, -- Timestamp for soft deletion
    p2p_synced_at TIMESTAMPTZ, -- Timestamp when this media file was last synced to P2P network. NULL means needs sync.
    p2p_shard_hash VARCHAR(255), -- Root hash or manifest hash of the object in P2P network (Blake3).
    p2p_encryption_key BYTEA, -- 32-byte encryption key used for sharding (needed for re-sharding during rebalance)
    p2p_encrypted_size INTEGER, -- Size of the encrypted blob before erasure coding (needed for reconstruction)
    PRIMARY KEY (deviceid, hash)
);

CREATE INDEX IF NOT EXISTS idx_video_deviceid_type_created_at ON videos(deviceid, type, created_at DESC);
-- Index for soft delete filtering
CREATE INDEX IF NOT EXISTS idx_videos_deleted_at ON videos(deleted_at) WHERE deleted_at IS NULL;
-- Index for verification status
CREATE INDEX IF NOT EXISTS idx_videos_verification_status ON videos(verification_status);
-- Composite for verification worker: WHERE deleted_at IS NULL AND (verification_status = 0/1/-1 ...)
CREATE INDEX IF NOT EXISTS idx_videos_verification_pending ON videos(verification_status, last_verified_at) WHERE deleted_at IS NULL;
-- Index for user_id queries
CREATE INDEX IF NOT EXISTS idx_videos_user_id ON videos(user_id);
-- Index for fast hash lookups (used in get_video, existence checks)
CREATE INDEX IF NOT EXISTS idx_videos_hash ON videos(hash);
-- Partial index for thumbnail queries (only index rows with thumbnails)
CREATE INDEX IF NOT EXISTS idx_videos_has_thumbnail ON videos(has_thumbnail) WHERE has_thumbnail = true;
-- Index for finding unsynced videos efficiently (P2P media replication)
CREATE INDEX IF NOT EXISTS idx_videos_need_sync ON videos(created_at) WHERE p2p_synced_at IS NULL;
-- Index for looking up P2P synced status
CREATE INDEX IF NOT EXISTS idx_videos_p2p_synced ON videos(p2p_synced_at);
-- Composite index for main gallery query: WHERE user_id = $N AND deleted_at IS NULL ORDER BY created_at DESC
CREATE INDEX IF NOT EXISTS idx_videos_user_created ON videos(user_id, created_at DESC) WHERE deleted_at IS NULL;

-- Persons table (face clusters representing individuals)
-- MOVED BEFORE FACES TO RESOLVE CIRCULAR DEPENDENCY
CREATE TABLE IF NOT EXISTS persons (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,

    -- Person name (optional, user-provided)
    name VARCHAR(255),

    -- Representative face embedding (centroid or best face embedding from cluster)
    representative_embedding vector(512),

    -- Representative face (for thumbnail display)
    -- Constraint added later to avoid circular dependency
    representative_face_id BIGINT,

    -- Metadata
    face_count INTEGER NOT NULL DEFAULT 0,  -- Number of faces in this cluster
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Indices for persons table
CREATE INDEX IF NOT EXISTS idx_persons_user_id ON persons(user_id);
-- Compound index for listing persons by user with priority ordering
CREATE INDEX IF NOT EXISTS idx_persons_user_list ON persons(user_id, face_count DESC, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_persons_name ON persons(name) WHERE name IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_persons_face_count ON persons(face_count DESC);
CREATE INDEX IF NOT EXISTS idx_persons_updated_at ON persons(updated_at DESC);

-- HNSW index for person similarity (for finding similar persons)
CREATE INDEX IF NOT EXISTS idx_persons_embedding_hnsw ON persons
USING hnsw (representative_embedding vector_cosine_ops)
WITH (m = 16, ef_construction = 64);

-- Face detection and recognition tables for person identification
CREATE TABLE IF NOT EXISTS faces (
    id BIGSERIAL PRIMARY KEY,
    image_hash VARCHAR(255) NOT NULL,
    image_deviceid VARCHAR(255) NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,

    -- Face location (bounding box in original image coordinates)
    bbox_x INTEGER NOT NULL,
    bbox_y INTEGER NOT NULL,
    bbox_width INTEGER NOT NULL,
    bbox_height INTEGER NOT NULL,

    -- Face embedding (512-dimensional from InsightFace)
    embedding vector(512) NOT NULL,

    -- Face quality metrics
    confidence REAL NOT NULL CONSTRAINT chk_faces_confidence CHECK (confidence >= 0.0 AND confidence <= 1.0),  -- Detection confidence (0.0-1.0)

    -- Person clustering
    person_id BIGINT REFERENCES persons(id) ON DELETE SET NULL,

    -- Metadata
    detected_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Foreign key to images table
    FOREIGN KEY (image_deviceid, image_hash) REFERENCES images(deviceid, hash) ON DELETE CASCADE
);

-- Indices for faces table
CREATE INDEX IF NOT EXISTS idx_faces_image ON faces(image_deviceid, image_hash);
CREATE INDEX IF NOT EXISTS idx_faces_user_id ON faces(user_id);
CREATE INDEX IF NOT EXISTS idx_faces_person_id ON faces(person_id);
-- Compound index for face clustering: WHERE user_id = $1 AND person_id IS NULL ORDER BY detected_at
CREATE INDEX IF NOT EXISTS idx_faces_user_unclustered ON faces(user_id, detected_at) WHERE person_id IS NULL;
CREATE INDEX IF NOT EXISTS idx_faces_detected_at ON faces(detected_at DESC);

-- HNSW index for face similarity search (same config as CLIP: m=16, ef_construction=64)
CREATE INDEX IF NOT EXISTS idx_faces_embedding_hnsw ON faces
USING hnsw (embedding vector_cosine_ops)
WITH (m = 16, ef_construction = 64);

-- Add circular foreign key constraint
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 
        FROM information_schema.table_constraints 
        WHERE constraint_name = 'fk_persons_representative_face' 
        AND table_name = 'persons'
    ) THEN
        ALTER TABLE persons
        ADD CONSTRAINT fk_persons_representative_face
        FOREIGN KEY (representative_face_id)
        REFERENCES faces(id)
        ON DELETE SET NULL;
    END IF;
END $$;

-- AI processing settings table (per-user settings)
-- This stores runtime-configurable AI processing preferences
CREATE TABLE IF NOT EXISTS ai_settings (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,

    -- AI description generation (using vision models)
    enable_ai_descriptions BOOLEAN NOT NULL DEFAULT TRUE,

    -- CLIP embeddings for semantic search
    enable_embeddings BOOLEAN NOT NULL DEFAULT TRUE,
    embedding_parallel_count INTEGER NOT NULL DEFAULT 10,

    -- Face detection and recognition
    enable_face_detection BOOLEAN NOT NULL DEFAULT TRUE,
    face_detection_parallel_count INTEGER NOT NULL DEFAULT 10,

    -- Backup settings
    enable_media_backup BOOLEAN NOT NULL DEFAULT FALSE,

    -- Metadata
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_ai_settings_user_id ON ai_settings(user_id);

-- Labels table for organizing media
CREATE TABLE IF NOT EXISTS labels (
    id SERIAL PRIMARY KEY,
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    color VARCHAR(7) DEFAULT '#808080', -- Hex color code
    created_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE(user_id, name)
);

CREATE INDEX IF NOT EXISTS idx_labels_user_id ON labels(user_id);

-- Many-to-many relationship between images and labels
CREATE TABLE IF NOT EXISTS image_labels (
    image_hash VARCHAR(255) NOT NULL,
    image_deviceid VARCHAR(255) NOT NULL,
    label_id INTEGER REFERENCES labels(id) ON DELETE CASCADE,
    PRIMARY KEY (image_hash, image_deviceid, label_id),
    FOREIGN KEY (image_deviceid, image_hash) REFERENCES images(deviceid, hash) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_image_labels_label_id ON image_labels(label_id);
CREATE INDEX IF NOT EXISTS idx_image_labels_image ON image_labels(image_deviceid, image_hash);
-- Composite for gallery label filter JOIN: ON t.hash = l.image_hash AND l.label_id = $N
CREATE INDEX IF NOT EXISTS idx_image_labels_hash_label ON image_labels(image_hash, label_id);

-- Many-to-many relationship between videos and labels
CREATE TABLE IF NOT EXISTS video_labels (
    video_hash VARCHAR(255) NOT NULL,
    video_deviceid VARCHAR(255) NOT NULL,
    label_id INTEGER REFERENCES labels(id) ON DELETE CASCADE,
    PRIMARY KEY (video_hash, video_deviceid, label_id),
    FOREIGN KEY (video_deviceid, video_hash) REFERENCES videos(deviceid, hash) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_video_labels_label_id ON video_labels(label_id);
CREATE INDEX IF NOT EXISTS idx_video_labels_video ON video_labels(video_deviceid, video_hash);
-- Composite for gallery label filter JOIN: ON t.hash = l.video_hash AND l.label_id = $N
CREATE INDEX IF NOT EXISTS idx_video_labels_hash_label ON video_labels(video_hash, label_id);

-- P2P Storage Nodes table
CREATE TABLE IF NOT EXISTS p2p_nodes (
    node_id VARCHAR(64) PRIMARY KEY, -- Ed25519 public key in hex
    public_addr VARCHAR(255),
    last_seen TIMESTAMPTZ DEFAULT NOW(),
    is_active BOOLEAN DEFAULT TRUE,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_p2p_nodes_last_seen ON p2p_nodes(last_seen);

-- P2P Shards table (maps media to distributed chunks)
CREATE TABLE IF NOT EXISTS p2p_shards (
    id BIGSERIAL PRIMARY KEY,
    file_hash VARCHAR(255) NOT NULL, -- Media hash (BLAKE3)
    shard_index INTEGER NOT NULL, -- 0 to 4 for 3/5 EC
    node_id VARCHAR(64) REFERENCES p2p_nodes(node_id) ON DELETE CASCADE,
    shard_hash VARCHAR(64) NOT NULL, -- BLAKE3 hash of the shard itself
    last_checked_at TIMESTAMPTZ, -- When this shard was last verified on the remote node
    created_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE(file_hash, shard_index)
);

CREATE INDEX IF NOT EXISTS idx_p2p_shards_file_hash ON p2p_shards(file_hash);
CREATE INDEX IF NOT EXISTS idx_p2p_shards_node_id ON p2p_shards(node_id);

-- Idempotent index migrations: recreate partial indexes to exclude soft-deleted rows.
-- IF NOT EXISTS won't update an existing index whose WHERE condition changed, so we drop+recreate.
DROP INDEX IF EXISTS idx_images_embedding_status;
CREATE INDEX IF NOT EXISTS idx_images_embedding_status ON images(embedding_generated_at)
WHERE embedding_generated_at IS NULL AND deleted_at IS NULL;

DROP INDEX IF EXISTS idx_images_face_detection_pending;
CREATE INDEX IF NOT EXISTS idx_images_face_detection_pending ON images(face_detection_completed_at)
WHERE face_detection_completed_at IS NULL AND deleted_at IS NULL;

DROP INDEX IF EXISTS idx_images_quality_pending;
CREATE INDEX IF NOT EXISTS idx_images_quality_pending ON images(quality_score_generated_at)
WHERE quality_score_generated_at IS NULL AND deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_images_description_pending ON images(created_at)
WHERE description IS NULL AND deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_images_verification_pending ON images(verification_status, last_verified_at)
WHERE deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_videos_verification_pending ON videos(verification_status, last_verified_at)
WHERE deleted_at IS NULL;
