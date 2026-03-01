//! Application constants and enums
//!
//! This module contains all the constants and enums used throughout the application
//! to avoid magic strings and improve type safety.

/// Media types for images and videos
pub mod media {
    pub const TYPE_ALL: &str = "all";
    pub const TYPE_CAMERA: &str = "camera";
    pub const TYPE_WHATSAPP: &str = "whatsapp";
    pub const TYPE_SCREENSHOT: &str = "screenshot";
    pub const TYPE_SCREEN_RECORDING: &str = "screen_recording";
    pub const TYPE_OTHER: &str = "other";
}

/// Database table names
pub mod tables {
    pub const IMAGES: &str = "images";
    pub const VIDEOS: &str = "videos";
    pub const STARRED_IMAGES: &str = "starred_images";
    pub const STARRED_VIDEOS: &str = "starred_videos";
    pub const USERS: &str = "users";
}

/// User roles
pub mod roles {
    pub const ADMIN: &str = "admin";
    pub const USER: &str = "user";
}

/// Verification status codes
pub mod verification {
    /// Not verified/pending
    pub const PENDING: i32 = 0;
    /// Verified/OK
    pub const VERIFIED: i32 = 1;
    /// Failed verification
    pub const FAILED: i32 = -1;
}

/// Default values for pagination
pub mod pagination {
    pub const DEFAULT_PAGE: usize = 1;
    pub const DEFAULT_LIMIT: usize = 50;
}

/// HTTP status messages
pub mod messages {
    pub const AUTH_REQUIRED: &str = "Authentication required";
    pub const IMAGE_NOT_FOUND: &str = "Image not found.";
    pub const VIDEO_NOT_FOUND: &str = "Video not found.";
    pub const THUMBNAIL_NOT_FOUND: &str = "Thumbnail not found.";
    pub const INVALID_TOKEN: &str = "Invalid token";
    pub const DATABASE_ERROR: &str = "Database error";
}
