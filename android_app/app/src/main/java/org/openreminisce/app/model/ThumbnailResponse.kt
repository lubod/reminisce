package org.openreminisce.app.model

data class ThumbnailResponse(
    val thumbnails: List<ThumbnailInfo>,
    val total: Int,
    val page: Int,
    val limit: Int
)