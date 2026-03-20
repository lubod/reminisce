package org.openreminisce.app.model

data class SearchResult(
    val hash: String,
    val name: String,
    val description: String? = null,
    val place: String? = null,
    val created_at: String,
    val similarity: Float,
    val starred: Boolean = false,
    val device_id: String? = null,
    val distance_km: Float? = null,
    val thumbnail_url: String? = null,
    val media_type: String = "image"
)

data class SearchResponse(
    val results: List<SearchResult>,
    val total: Int,
    val query: String,
    val min_similarity: Float,
    val search_mode: String
)
