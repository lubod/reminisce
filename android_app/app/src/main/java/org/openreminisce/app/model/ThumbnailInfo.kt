package org.openreminisce.app.model

data class ThumbnailInfo(
    val hash: String,
    val created_at: String,  // This will be in ISO 8601 format like "2025-09-26T12:06:40Z"
    val place: String? = null,  // Optional location/place information
    val mediaType: String = "image"  // "image" or "video", defaults to "image"
)