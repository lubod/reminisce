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
    val similarity: Float? = null,
    val thumbnailUrl: String? = null,
    val fileSizeBytes: Long? = null,
    val aestheticScore: Float? = null,
    val orientation: Int? = null
)

data class MapPoint(
    val hash: String,
    val lon: Double,
    val lat: Double,
    val created_at: String,
    val place: String? = null,
    val starred: Boolean = false,
    val device_id: String? = null,
    val has_thumbnail: Boolean = true
)

data class MapPointsResponse(
    val points: List<MapPoint>,
    val total: Int
)

data class EnhanceResult(
    val imageBytes: ByteArray,
    val operations: List<String>
) {
    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (other !is EnhanceResult) return false
        return imageBytes.contentEquals(other.imageBytes) && operations == other.operations
    }

    override fun hashCode(): Int {
        var result = imageBytes.contentHashCode()
        result = 31 * result + operations.hashCode()
        return result
    }
}