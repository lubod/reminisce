package org.openreminisce.app.model

sealed class MediaItem {
    data class DateHeader(val date: String, val place: String? = null) : MediaItem()
    data class Image(val imageInfo: ImageInfo) : MediaItem()
}