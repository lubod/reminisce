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
    /** Raw EXIF-style orientation value (1-8); null when unknown. */
    val orientation: Int? = null,
    /** "Landscape" / "Portrait" / "Square" as computed by the server. */
    val orientation_label: String? = null,
    /** Displayed resolution "W × H" after rotation, e.g. "3000 × 4000". */
    val resolution_label: String? = null,
    val media_type: String? = null
)
