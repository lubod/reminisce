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
    val locationRadiusKm: Float = 10f,
    val searchMode: SearchMode = SearchMode.SEMANTIC,
    val minSimilarity: Float = 0.08f,
    val sortBy: String = "date",   // "date" | "size" | "quality"
    val sortOrder: String = "desc", // "desc" | "asc"
    val groupByPlace: Boolean = false
) {
    fun isDefault(): Boolean = this == MediaFilter()

    fun activeFilterCount(): Int {
        var count = 0
        if (mediaType != MediaTypeFilter.ALL) count++
        if (starredOnly) count++
        if (startDate != null || endDate != null) count++
        if (labelId != null) count++
        if (!deviceId.isNullOrBlank()) count++
        if (locationLat != null && locationLon != null) count++
        if (sortBy != "date" || sortOrder != "desc") count++
        if (groupByPlace) count++
        return count
    }
}

enum class MediaTypeFilter { ALL, IMAGE, VIDEO }

enum class SearchMode { SEMANTIC, TEXT, HYBRID }
