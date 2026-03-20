package org.openreminisce.app.model

data class MediaFilter(
    val mediaType: MediaTypeFilter = MediaTypeFilter.ALL,
    val starredOnly: Boolean = false,
    val startDate: Long? = null,    // Unix timestamp ms
    val endDate: Long? = null,      // Unix timestamp ms
    val labelId: Int? = null,
    val deviceId: String? = null,
    val locationLat: Double? = null,
    val locationLon: Double? = null,
    val locationRadiusKm: Float = 50f,
    val searchMode: SearchMode = SearchMode.SEMANTIC,
    val minSimilarity: Float = 0.08f
) {
    fun isDefault(): Boolean = this == MediaFilter()
}

enum class MediaTypeFilter { ALL, IMAGE, VIDEO }

enum class SearchMode { SEMANTIC, TEXT, HYBRID }
