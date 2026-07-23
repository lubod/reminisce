package org.openreminisce.app.model

data class ImageMetadata(
    val hash: String,
    val name: String,
    val description: String? = null,
    val place: String? = null,
    val created_at: String,
    val exif: String? = null,  // Raw JSON string
    val starred: Boolean = false,
    val device_id: String? = null,
    val file_size_bytes: Long? = null,
    val width: Int? = null,
    val height: Int? = null,
    val media_type: String? = null
)
