/// Custom business metrics for the Reminisce application
///
/// This module defines Prometheus metrics for tracking application-specific
/// events and behaviors beyond standard HTTP metrics.

use prometheus::{
    IntCounter, IntGauge, Histogram, HistogramOpts,
    register_int_counter, register_int_gauge, register_histogram,
};
use once_cell::sync::Lazy;

// ============================================================================
// User Metrics
// ============================================================================

/// Total number of user registrations
pub static USER_REGISTRATIONS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "user_registrations_total",
        "Total number of user registrations"
    ).expect("Failed to register user_registrations_total metric")
});

/// Total number of user logins
pub static USER_LOGINS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "user_logins_total",
        "Total number of successful user logins"
    ).expect("Failed to register user_logins_total metric")
});

/// Total number of failed login attempts
pub static USER_LOGIN_FAILURES_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "user_login_failures_total",
        "Total number of failed login attempts"
    ).expect("Failed to register user_login_failures_total metric")
});

/// Currently active user sessions
pub static ACTIVE_SESSIONS: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "active_sessions",
        "Number of currently active user sessions"
    ).expect("Failed to register active_sessions metric")
});

// ============================================================================
// Memory/Content Metrics
// ============================================================================

/// Total number of memories created
pub static MEMORIES_CREATED_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "memories_created_total",
        "Total number of memories created"
    ).expect("Failed to register memories_created_total metric")
});

/// Total number of memories retrieved
pub static MEMORIES_RETRIEVED_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "memories_retrieved_total",
        "Total number of memories retrieved"
    ).expect("Failed to register memories_retrieved_total metric")
});

/// Total number of memories deleted
pub static MEMORIES_DELETED_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "memories_deleted_total",
        "Total number of memories deleted"
    ).expect("Failed to register memories_deleted_total metric")
});

/// Total number of memories shared
pub static MEMORIES_SHARED_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "memories_shared_total",
        "Total number of memories shared"
    ).expect("Failed to register memories_shared_total metric")
});

// ============================================================================
// Database Metrics
// ============================================================================

/// Database query duration histogram
pub static DB_QUERY_DURATION: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        HistogramOpts::new(
            "db_query_duration_seconds",
            "Database query execution time in seconds"
        )
        .buckets(vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0])
    ).expect("Failed to register db_query_duration_seconds metric")
});

/// Total number of database connection errors
pub static DB_CONNECTION_ERRORS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "db_connection_errors_total",
        "Total number of database connection errors"
    ).expect("Failed to register db_connection_errors_total metric")
});

/// Total number of slow queries (>100ms)
pub static SLOW_QUERIES_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "slow_queries_total",
        "Total number of slow database queries (>100ms)"
    ).expect("Failed to register slow_queries_total metric")
});

// ============================================================================
// P2P/Network Metrics
// ============================================================================

/// Total number of P2P connections established
pub static P2P_CONNECTIONS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "p2p_connections_total",
        "Total number of P2P connections established"
    ).expect("Failed to register p2p_connections_total metric")
});

/// Currently active P2P sessions
pub static ACTIVE_P2P_SESSIONS: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "active_p2p_sessions",
        "Number of currently active P2P sessions"
    ).expect("Failed to register active_p2p_sessions metric")
});

/// Successful NAT traversals
pub static NAT_TRAVERSAL_SUCCESS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "nat_traversal_success_total",
        "Total number of successful NAT traversals"
    ).expect("Failed to register nat_traversal_success_total metric")
});

/// Failed NAT traversals
pub static NAT_TRAVERSAL_FAILURES_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "nat_traversal_failures_total",
        "Total number of failed NAT traversals"
    ).expect("Failed to register nat_traversal_failures_total metric")
});

// ============================================================================
// File/Upload Metrics
// ============================================================================

/// Total number of file uploads
pub static FILE_UPLOADS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "file_uploads_total",
        "Total number of file uploads"
    ).expect("Failed to register file_uploads_total metric")
});

/// Total bytes uploaded
pub static BYTES_UPLOADED_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "bytes_uploaded_total",
        "Total number of bytes uploaded"
    ).expect("Failed to register bytes_uploaded_total metric")
});

/// Upload duration histogram
pub static UPLOAD_DURATION: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        HistogramOpts::new(
            "upload_duration_seconds",
            "File upload duration in seconds"
        )
        .buckets(vec![0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0])
    ).expect("Failed to register upload_duration_seconds metric")
});

// ============================================================================
// DB Pool Metrics
// ============================================================================

pub static DB_POOL_SIZE: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "db_pool_size",
        "Current total number of connections in the pool"
    ).expect("Failed to register db_pool_size")
});

pub static DB_POOL_AVAILABLE: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "db_pool_available",
        "Number of available connections in the pool"
    ).expect("Failed to register db_pool_available")
});

pub static DB_POOL_ACTIVE: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "db_pool_active",
        "Number of active (in-use) connections in the pool"
    ).expect("Failed to register db_pool_active")
});

pub static DB_POOL_MAX_SIZE: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "db_pool_max_size",
        "Configured maximum size of the pool"
    ).expect("Failed to register db_pool_max_size")
});

pub static DB_POOL_UTILIZATION: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "db_pool_utilization_percent",
        "Percentage of pool currently in use"
    ).expect("Failed to register db_pool_utilization_percent")
});

// ============================================================================
// Error Metrics
// ============================================================================

/// Total number of application errors by type
pub static APPLICATION_ERRORS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "application_errors_total",
        "Total number of application errors"
    ).expect("Failed to register application_errors_total metric")
});

/// Total number of validation errors
pub static VALIDATION_ERRORS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "validation_errors_total",
        "Total number of validation errors"
    ).expect("Failed to register validation_errors_total metric")
});

// ============================================================================
// Backup Metrics
// ============================================================================

/// Total number of backup attempts
pub static BACKUP_ATTEMPTS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "backup_attempts_total",
        "Total number of backup cycle attempts"
    ).expect("Failed to register backup_attempts_total metric")
});

/// Total number of successful backups
pub static BACKUP_SUCCESS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "backup_success_total",
        "Total number of successful backup transfers"
    ).expect("Failed to register backup_success_total metric")
});

/// Total number of failed backups
pub static BACKUP_FAILURES_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "backup_failures_total",
        "Total number of failed backup transfers"
    ).expect("Failed to register backup_failures_total metric")
});

/// Backup size in bytes
pub static BACKUP_SIZE_BYTES: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        HistogramOpts::new(
            "backup_size_bytes",
            "Size of encrypted backups in bytes"
        )
        .buckets(vec![
            1_000_000.0,      // 1 MB
            5_000_000.0,      // 5 MB
            10_000_000.0,     // 10 MB
            50_000_000.0,     // 50 MB
            100_000_000.0,    // 100 MB
            500_000_000.0,    // 500 MB
        ])
    ).expect("Failed to register backup_size_bytes metric")
});

/// Backup duration in seconds
pub static BACKUP_DURATION_SECONDS: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        HistogramOpts::new(
            "backup_duration_seconds",
            "Duration of backup cycle in seconds"
        )
        .buckets(vec![1.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0])
    ).expect("Failed to register backup_duration_seconds metric")
});

/// Number of peers currently available for backup
pub static BACKUP_PEERS_AVAILABLE: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "backup_peers_available",
        "Number of peers currently available for backup storage"
    ).expect("Failed to register backup_peers_available metric")
});

/// Number of backups deduplicated (hash match)
pub static BACKUP_DEDUPLICATED_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "backup_deduplicated_total",
        "Total number of backups deduplicated via hash matching"
    ).expect("Failed to register backup_deduplicated_total metric")
});

/// Number of backup rate limit hits
pub static BACKUP_RATE_LIMITED_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "backup_rate_limited_total",
        "Total number of times backup was rate limited"
    ).expect("Failed to register backup_rate_limited_total metric")
});

// ============================================================================
// Processing Pipeline Metrics
// ============================================================================

/// Total number of images in the library
pub static TOTAL_IMAGES: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "total_images",
        "Total number of images in the library"
    ).expect("Failed to register total_images metric")
});

/// Total number of images with embeddings
pub static IMAGES_WITH_EMBEDDING: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "images_with_embedding",
        "Total number of images with embeddings generated"
    ).expect("Failed to register images_with_embedding metric")
});

/// Total number of images with descriptions
pub static IMAGES_WITH_DESCRIPTION: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "images_with_description",
        "Total number of images with descriptions generated"
    ).expect("Failed to register images_with_description metric")
});

/// Total number of images that have been processed for faces
pub static IMAGES_FACE_PROCESSED: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "images_face_processed",
        "Total number of images processed for face detection"
    ).expect("Failed to register images_face_processed metric")
});

/// File verification duration histogram
pub static VERIFICATION_DURATION: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        HistogramOpts::new(
            "verification_duration_seconds",
            "File verification (BLAKE3 hash) duration in seconds"
        )
        .buckets(vec![0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0])
    ).expect("Failed to register verification_duration_seconds metric")
});

/// Total successful verifications
pub static VERIFICATION_SUCCESS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "verification_success_total",
        "Total number of successful file verifications"
    ).expect("Failed to register verification_success_total metric")
});

/// Total failed verifications
pub static VERIFICATION_FAILURES_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "verification_failures_total",
        "Total number of failed file verifications"
    ).expect("Failed to register verification_failures_total metric")
});

/// AI description generation duration histogram
pub static AI_DESCRIPTION_DURATION: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        HistogramOpts::new(
            "ai_description_duration_seconds",
            "AI description generation duration in seconds"
        )
        .buckets(vec![0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0])
    ).expect("Failed to register ai_description_duration_seconds metric")
});

/// Total successful AI descriptions
pub static AI_DESCRIPTION_SUCCESS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "ai_description_success_total",
        "Total number of successful AI description generations"
    ).expect("Failed to register ai_description_success_total metric")
});

/// Total failed AI descriptions
pub static AI_DESCRIPTION_FAILURES_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "ai_description_failures_total",
        "Total number of failed AI description generations"
    ).expect("Failed to register ai_description_failures_total metric")
});

/// Embedding generation duration histogram
pub static EMBEDDING_DURATION: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        HistogramOpts::new(
            "embedding_duration_seconds",
            "Image embedding generation duration in seconds"
        )
        .buckets(vec![0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0])
    ).expect("Failed to register embedding_duration_seconds metric")
});

/// Total successful embeddings
pub static EMBEDDING_SUCCESS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "embedding_success_total",
        "Total number of successful embedding generations"
    ).expect("Failed to register embedding_success_total metric")
});

/// Total failed embeddings
pub static EMBEDDING_FAILURES_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "embedding_failures_total",
        "Total number of failed embedding generations"
    ).expect("Failed to register embedding_failures_total metric")
});

/// Face detection duration histogram
pub static FACE_DETECTION_DURATION: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        HistogramOpts::new(
            "face_detection_duration_seconds",
            "Face detection duration in seconds"
        )
        .buckets(vec![0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0])
    ).expect("Failed to register face_detection_duration_seconds metric")
});

/// Total successful face detections
pub static FACE_DETECTION_SUCCESS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "face_detection_success_total",
        "Total number of successful face detection runs"
    ).expect("Failed to register face_detection_success_total metric")
});

/// Total failed face detections
pub static FACE_DETECTION_FAILURES_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "face_detection_failures_total",
        "Total number of failed face detection runs"
    ).expect("Failed to register face_detection_failures_total metric")
});

/// Total faces detected
pub static FACES_DETECTED_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "faces_detected_total",
        "Total number of faces detected across all images"
    ).expect("Failed to register faces_detected_total metric")
});

/// Face clustering duration histogram
pub static FACE_CLUSTERING_DURATION: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        HistogramOpts::new(
            "face_clustering_duration_seconds",
            "Face clustering duration in seconds"
        )
        .buckets(vec![0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0])
    ).expect("Failed to register face_clustering_duration_seconds metric")
});

/// Thumbnail generation duration histogram
pub static THUMBNAIL_DURATION: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        HistogramOpts::new(
            "thumbnail_duration_seconds",
            "Thumbnail generation duration in seconds"
        )
        .buckets(vec![0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0])
    ).expect("Failed to register thumbnail_duration_seconds metric")
});

/// Total successful thumbnail generations
pub static THUMBNAIL_SUCCESS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "thumbnail_success_total",
        "Total number of successful thumbnail generations"
    ).expect("Failed to register thumbnail_success_total metric")
});

/// Total failed thumbnail generations
pub static THUMBNAIL_FAILURES_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "thumbnail_failures_total",
        "Total number of failed thumbnail generations"
    ).expect("Failed to register thumbnail_failures_total metric")
});

// ============================================================================
// Processing Delay Metrics (End-to-End Latency)
// ============================================================================

/// Thumbnail processing delay (time from upload to thumbnail ready)
pub static THUMBNAIL_PROCESSING_DELAY: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        HistogramOpts::new(
            "thumbnail_processing_delay_seconds",
            "Time from image upload to thumbnail generation completion in seconds"
        )
        .buckets(vec![1.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0, 3600.0])
    ).expect("Failed to register thumbnail_processing_delay_seconds metric")
});

/// AI description processing delay (time from upload to description ready)
pub static AI_DESCRIPTION_PROCESSING_DELAY: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        HistogramOpts::new(
            "ai_description_processing_delay_seconds",
            "Time from image upload to AI description completion in seconds"
        )
        .buckets(vec![5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0, 3600.0])
    ).expect("Failed to register ai_description_processing_delay_seconds metric")
});

/// Embedding processing delay (time from upload to embedding ready)
pub static EMBEDDING_PROCESSING_DELAY: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        HistogramOpts::new(
            "embedding_processing_delay_seconds",
            "Time from image upload to embedding completion in seconds"
        )
        .buckets(vec![5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0, 3600.0])
    ).expect("Failed to register embedding_processing_delay_seconds metric")
});

/// Face detection processing delay (time from upload to detection ready)
pub static FACE_DETECTION_PROCESSING_DELAY: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        HistogramOpts::new(
            "face_detection_processing_delay_seconds",
            "Time from image upload to face detection completion in seconds"
        )
        .buckets(vec![5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0, 3600.0])
    ).expect("Failed to register face_detection_processing_delay_seconds metric")
});


// ============================================================================
// Duplicate Detection Metrics
// ============================================================================

pub static DUPLICATE_PAIRS_TOTAL: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "duplicate_pairs_total",
        "Total known duplicate image pairs across all users"
    ).expect("Failed to register duplicate_pairs_total metric")
});

pub static DUPLICATE_CHECKED_IMAGES: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "duplicate_checked_images",
        "Number of images that have been checked for duplicates"
    ).expect("Failed to register duplicate_checked_images metric")
});

// ============================================================================
// P2P Shard Audit Metrics
// ============================================================================

pub static P2P_SHARDS_AUDITED_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "p2p_shards_audited_total",
        "Total P2P shards verified for integrity"
    ).expect("Failed to register p2p_shards_audited_total metric")
});

pub static P2P_SHARDS_REPAIRED_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "p2p_shards_repaired_total",
        "Total P2P shards successfully repaired"
    ).expect("Failed to register p2p_shards_repaired_total metric")
});

pub static P2P_SHARDS_REPAIR_FAILED_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "p2p_shards_repair_failed_total",
        "Total P2P shard repairs that failed"
    ).expect("Failed to register p2p_shards_repair_failed_total metric")
});

pub static P2P_ORPHANED_SHARDS_CLEANED_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "p2p_orphaned_shards_cleaned_total",
        "Orphaned P2P shard rows purged for deleted files"
    ).expect("Failed to register p2p_orphaned_shards_cleaned_total metric")
});

// ============================================================================
// Startup Registration
// ============================================================================

/// Force-register every Lazy metric so they appear in /metrics at 0 from startup.
/// Without this, counters and histograms only appear after their first increment,
/// which causes Grafana dashboards to show "no data" instead of 0.
pub fn init_metrics() {
    Lazy::force(&USER_REGISTRATIONS_TOTAL);
    Lazy::force(&USER_LOGINS_TOTAL);
    Lazy::force(&USER_LOGIN_FAILURES_TOTAL);
    Lazy::force(&ACTIVE_SESSIONS);
    Lazy::force(&MEMORIES_CREATED_TOTAL);
    Lazy::force(&MEMORIES_RETRIEVED_TOTAL);
    Lazy::force(&MEMORIES_DELETED_TOTAL);
    Lazy::force(&MEMORIES_SHARED_TOTAL);
    Lazy::force(&DB_QUERY_DURATION);
    Lazy::force(&DB_CONNECTION_ERRORS_TOTAL);
    Lazy::force(&SLOW_QUERIES_TOTAL);
    Lazy::force(&P2P_CONNECTIONS_TOTAL);
    Lazy::force(&ACTIVE_P2P_SESSIONS);
    Lazy::force(&NAT_TRAVERSAL_SUCCESS_TOTAL);
    Lazy::force(&NAT_TRAVERSAL_FAILURES_TOTAL);
    Lazy::force(&FILE_UPLOADS_TOTAL);
    Lazy::force(&BYTES_UPLOADED_TOTAL);
    Lazy::force(&UPLOAD_DURATION);
    Lazy::force(&APPLICATION_ERRORS_TOTAL);
    Lazy::force(&VALIDATION_ERRORS_TOTAL);
    Lazy::force(&BACKUP_ATTEMPTS_TOTAL);
    Lazy::force(&BACKUP_SUCCESS_TOTAL);
    Lazy::force(&BACKUP_FAILURES_TOTAL);
    Lazy::force(&BACKUP_SIZE_BYTES);
    Lazy::force(&BACKUP_DURATION_SECONDS);
    Lazy::force(&BACKUP_PEERS_AVAILABLE);
    Lazy::force(&BACKUP_DEDUPLICATED_TOTAL);
    Lazy::force(&BACKUP_RATE_LIMITED_TOTAL);
    Lazy::force(&VERIFICATION_DURATION);
    Lazy::force(&VERIFICATION_SUCCESS_TOTAL);
    Lazy::force(&VERIFICATION_FAILURES_TOTAL);
    Lazy::force(&AI_DESCRIPTION_DURATION);
    Lazy::force(&AI_DESCRIPTION_SUCCESS_TOTAL);
    Lazy::force(&AI_DESCRIPTION_FAILURES_TOTAL);
    Lazy::force(&EMBEDDING_DURATION);
    Lazy::force(&EMBEDDING_SUCCESS_TOTAL);
    Lazy::force(&EMBEDDING_FAILURES_TOTAL);
    Lazy::force(&FACE_DETECTION_DURATION);
    Lazy::force(&FACE_DETECTION_SUCCESS_TOTAL);
    Lazy::force(&FACE_DETECTION_FAILURES_TOTAL);
    Lazy::force(&FACES_DETECTED_TOTAL);
    Lazy::force(&FACE_CLUSTERING_DURATION);
    Lazy::force(&THUMBNAIL_DURATION);
    Lazy::force(&THUMBNAIL_SUCCESS_TOTAL);
    Lazy::force(&THUMBNAIL_FAILURES_TOTAL);
    Lazy::force(&THUMBNAIL_PROCESSING_DELAY);
    Lazy::force(&AI_DESCRIPTION_PROCESSING_DELAY);
    Lazy::force(&EMBEDDING_PROCESSING_DELAY);
    Lazy::force(&FACE_DETECTION_PROCESSING_DELAY);
    Lazy::force(&DUPLICATE_PAIRS_TOTAL);
    Lazy::force(&DUPLICATE_CHECKED_IMAGES);
    Lazy::force(&P2P_SHARDS_AUDITED_TOTAL);
    Lazy::force(&P2P_SHARDS_REPAIRED_TOTAL);
    Lazy::force(&P2P_SHARDS_REPAIR_FAILED_TOTAL);
    Lazy::force(&P2P_ORPHANED_SHARDS_CLEANED_TOTAL);
}
