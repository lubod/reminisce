package org.openreminisce.app.model

import java.util.Date
import java.io.Serializable

data class ImageInfo(
    val id: String,
    val date: Date,
    val thumbnailPath: String? = null,
    val place: String? = null,  // Optional location/place information
    val isBackedUp: Boolean = false,  // Whether this media is backed up to server
    val mediaType: String = "image",  // "image" or "video"
    val hash: String? = null,  // SHA-256 hash of the file
    val displayName: String? = null, // Cached display name to avoid repeated DB queries
    val relativePath: String? = null // Cached relative path (e.g. "DCIM/Camera/")
) : Serializable