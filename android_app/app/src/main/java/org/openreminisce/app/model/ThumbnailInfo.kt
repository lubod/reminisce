package org.openreminisce.app.model

data class ThumbnailInfo(
    val hash: String,
    val created_at: String,  // ISO 8601 format like "2025-09-26T12:06:40Z"
    val place: String? = null,
    val mediaType: String = "image",  // "image" or "video"
    val name: String = "",
    val starred: Boolean = false,
    val device_id: String? = null,
    val distance_km: Float? = null,
    val similarity: Float? = null
)