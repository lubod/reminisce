package org.openreminisce.app.repository

import android.content.Context
import android.net.Uri
import android.util.Log
import org.openreminisce.app.model.ImageInfo
import org.openreminisce.app.model.MediaItem
import org.openreminisce.app.util.DatabaseHelper
import org.openreminisce.app.util.HashCalculator
import org.openreminisce.app.util.MediaHelper
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

class LocalMediaRepository {
    companion object {
        private const val TAG = "LocalMediaRepository"
    }

    /**
     * Loads all local media (images and videos) from the device.
     * Returns a list of MediaItem objects sorted by date with date headers.
     * Checks backup status by verifying if file hash exists in database.
     */
    suspend fun loadLocalMediaWithBackupStatus(context: Context): List<MediaItem> {
        try {
            Log.d(TAG, "Loading local media...")

            val databaseHelper = DatabaseHelper(context)

            // Load images and videos with media type tags
            val images = MediaHelper.getAllImages(context).map { info ->
                val isBackedUp = checkBackupStatus(info, databaseHelper)
                info.copy(mediaType = "image", isBackedUp = isBackedUp)
            }

            val videos = MediaHelper.getAllVideos(context).map { info ->
                val isBackedUp = checkBackupStatus(info, databaseHelper)
                info.copy(mediaType = "video", isBackedUp = isBackedUp)
            }

            Log.d(TAG, "Loaded ${images.size} images and ${videos.size} videos")

            // Merge and sort media
            val mediaItems = mergeAndSortMedia(images, videos)

            Log.d(TAG, "Created ${mediaItems.size} media items (including headers)")

            return mediaItems
        } catch (e: Exception) {
            Log.e(TAG, "Error loading local media", e)
            return emptyList()
        }
    }

    /**
     * Checks if a file is backed up by looking for its hash in the database.
     * Returns true if the file hash exists in database (indicating it's been backed up).
     */
    private fun checkBackupStatus(
        imageInfo: ImageInfo,
        databaseHelper: DatabaseHelper
    ): Boolean {
        try {
            val uri = Uri.parse(imageInfo.id)
            val fileId = uri.toString()

            // Check if hash exists in database
            val cachedHash = databaseHelper.getHash(fileId)
            if (cachedHash != null) {
                Log.d(TAG, "File $fileId is backed up (hash found in cache)")
                return true
            }

            // If not in cache, we could calculate hash and check server,
            // but that's expensive. For now, just return false.
            // TODO: Implement lazy hash calculation and server check
            return false
        } catch (e: Exception) {
            Log.e(TAG, "Error checking backup status for ${imageInfo.id}", e)
            return false
        }
    }

    /**
     * Merges images and videos, sorts by date descending, and groups by date with headers.
     */
    private fun mergeAndSortMedia(
        images: List<ImageInfo>,
        videos: List<ImageInfo>
    ): List<MediaItem> {
        val allMedia = mutableListOf<ImageInfo>()
        allMedia.addAll(images)
        allMedia.addAll(videos)

        // Sort by date descending (newest first)
        allMedia.sortByDescending { it.date }

        // Group by date and create MediaItem list with headers
        val mediaItems = mutableListOf<MediaItem>()
        var currentDate: String? = null
        val dateFormat = SimpleDateFormat("MMM dd, yyyy", Locale.getDefault())

        for (media in allMedia) {
            val dateString = dateFormat.format(media.date)

            // Add date header if date changed
            if (dateString != currentDate) {
                mediaItems.add(MediaItem.DateHeader(dateString, media.place))
                currentDate = dateString
            }

            // Add media item
            mediaItems.add(MediaItem.Image(media))
        }

        return mediaItems
    }
}
