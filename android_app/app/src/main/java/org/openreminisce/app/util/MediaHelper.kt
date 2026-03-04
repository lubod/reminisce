package org.openreminisce.app.util

import android.content.ContentUris
import android.content.Context
import android.database.Cursor
import android.net.Uri
import android.os.Build
import android.provider.MediaStore
import android.util.Log
import org.openreminisce.app.model.ImageInfo
import org.openreminisce.app.util.ThumbnailHelper
import java.io.File

class MediaHelper {
    companion object {
        private const val TAG = "MediaHelper"

        /**
         * Common method to query media of any type from MediaStore.
         * This eliminates code duplication between getAllImages and getAllVideos.
         */
        private fun getAllMediaOfType(
            context: Context,
            mediaUri: Uri,
            mediaType: String
        ): List<ImageInfo> {
            val mediaList = mutableListOf<ImageInfo>()

            // Determine path column based on API level
            val pathColumn = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                MediaStore.MediaColumns.RELATIVE_PATH
            } else {
                MediaStore.MediaColumns.DATA
            }

            // Common projection for both images and videos
            val projection = arrayOf(
                MediaStore.MediaColumns._ID,
                MediaStore.MediaColumns.DATE_ADDED,
                MediaStore.MediaColumns.DISPLAY_NAME,
                pathColumn
            )

            val sortOrder = "${MediaStore.MediaColumns.DATE_ADDED} DESC"

            try {
                val cursor: Cursor? = context.contentResolver.query(
                    mediaUri,
                    projection,
                    null,
                    null,
                    sortOrder
                )

                Log.d(TAG, "$mediaType cursor query result: ${cursor?.count ?: 0} items")

                cursor?.use {
                    val idColumn = it.getColumnIndexOrThrow(MediaStore.MediaColumns._ID)
                    val dateColumn = it.getColumnIndexOrThrow(MediaStore.MediaColumns.DATE_ADDED)
                    val nameColumn = it.getColumnIndexOrThrow(MediaStore.MediaColumns.DISPLAY_NAME)
                    val pathColumnIndex = it.getColumnIndex(pathColumn)

                    var count = 0
                    while (it.moveToNext()) {
                        val id = it.getLong(idColumn)
                        val dateAdded = it.getLong(dateColumn)
                        val fileName = it.getString(nameColumn)
                        
                        // Extract relative path
                        var relativePath: String? = null
                        if (pathColumnIndex >= 0) {
                             val rawPath = it.getString(pathColumnIndex)
                             if (rawPath != null) {
                                 if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                                     relativePath = rawPath
                                 } else {
                                     // For older Android, extract relative path from full path (DATA)
                                     try {
                                         val file = File(rawPath)
                                         val parentPath = file.parent
                                         if (parentPath != null) {
                                             val storagePath = android.os.Environment.getExternalStorageDirectory().absolutePath
                                             relativePath = if (parentPath.startsWith(storagePath)) {
                                                 parentPath.substring(storagePath.length).removePrefix("/") + "/"
                                             } else {
                                                 parentPath + "/"
                                             }
                                         }
                                     } catch (e: Exception) {
                                         // Keep null if parsing fails
                                     }
                                 }
                             }
                        }

                        val contentUri = ContentUris.withAppendedId(mediaUri, id)

                        // Log only first 10 to reduce noise
                        if (count < 10) {
                            Log.d(TAG, "Found $mediaType #${count + 1}: $fileName, ID: $id, URI: $contentUri")
                        }

                        mediaList.add(ImageInfo(
                            id = contentUri.toString(), 
                            date = java.util.Date(dateAdded * 1000), 
                            thumbnailPath = null,
                            displayName = fileName,
                            relativePath = relativePath,
                            mediaType = if (mediaType == "videos") "video" else "image"
                        ))
                        count++
                    }
                    Log.d(TAG, "Total $mediaType added to list: ${mediaList.size}")
                }
            } catch (e: Exception) {
                Log.e(TAG, "Error getting $mediaType", e)
            }

            Log.d(TAG, "Final $mediaType list size: ${mediaList.size}")
            return mediaList
        }

        fun getAllImages(context: Context): List<ImageInfo> {
            return getAllMediaOfType(
                context,
                MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
                "images"
            )
        }

        fun getAllVideos(context: Context): List<ImageInfo> {
            return getAllMediaOfType(
                context,
                MediaStore.Video.Media.EXTERNAL_CONTENT_URI,
                "videos"
            )
        }
        
        /**
         * Gets file path from URI using deprecated DATA column.
         * DEPRECATED: Use getFileSizeFromUri and openInputStream instead for Android 10+ compatibility.
         * This method may fail on Android 10+ due to Scoped Storage restrictions.
         */
        @Deprecated("Use URI-based methods instead", ReplaceWith("getFileSizeFromUri and ContentResolver.openInputStream"))
        fun getFilePathFromUri(context: Context, uri: Uri): String? {
            var filePath: String? = null
            val projection = arrayOf(MediaStore.Images.Media.DATA)

            try {
                val cursor = context.contentResolver.query(uri, projection, null, null, null)
                cursor?.use {
                    if (it.moveToFirst()) {
                        val columnIndex = it.getColumnIndexOrThrow(MediaStore.Images.Media.DATA)
                        filePath = it.getString(columnIndex)
                    }
                }
            } catch (e: Exception) {
                Log.e(TAG, "Error getting file path from URI", e)
            }

            return filePath
        }

        /**
         * Gets the file size from a URI without requiring direct file path access.
         * Works on all Android versions including Android 10+ with Scoped Storage.
         */
        fun getFileSizeFromUri(context: Context, uri: Uri): Long? {
            try {
                context.contentResolver.query(uri, null, null, null, null)?.use { cursor ->
                    if (cursor.moveToFirst()) {
                        val sizeIndex = cursor.getColumnIndex(MediaStore.MediaColumns.SIZE)
                        if (sizeIndex >= 0) {
                            return cursor.getLong(sizeIndex)
                        }
                    }
                }
            } catch (e: Exception) {
                Log.e(TAG, "Error getting file size from URI: $uri", e)
            }
            return null
        }

        /**
         * Gets the display name (filename) from a URI.
         * Works on all Android versions including Android 10+ with Scoped Storage.
         */
        fun getDisplayNameFromUri(context: Context, uri: Uri): String? {
            try {
                context.contentResolver.query(uri, null, null, null, null)?.use { cursor ->
                    if (cursor.moveToFirst()) {
                        val nameIndex = cursor.getColumnIndex(MediaStore.MediaColumns.DISPLAY_NAME)
                        if (nameIndex >= 0) {
                            return cursor.getString(nameIndex)
                        }
                    }
                }
            } catch (e: Exception) {
                Log.e(TAG, "Error getting display name from URI: $uri", e)
            }
            return null
        }

        /**
         * Gets the relative path (folder structure) from a URI.
         * On Android 10+, returns the RELATIVE_PATH (e.g., "DCIM/Camera/").
         * On older versions, extracts path from DATA column.
         * Returns null if path cannot be determined.
         */
        fun getRelativePathFromUri(context: Context, uri: Uri): String? {
            try {
                context.contentResolver.query(uri, null, null, null, null)?.use { cursor ->
                    if (cursor.moveToFirst()) {
                        // Try RELATIVE_PATH first (Android 10+)
                        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                            val relativePathIndex = cursor.getColumnIndex(MediaStore.MediaColumns.RELATIVE_PATH)
                            if (relativePathIndex >= 0) {
                                val relativePath = cursor.getString(relativePathIndex)
                                if (!relativePath.isNullOrEmpty()) {
                                    return relativePath
                                }
                            }
                        }

                        // Fallback to DATA column (older Android versions)
                        val dataIndex = cursor.getColumnIndex(MediaStore.MediaColumns.DATA)
                        if (dataIndex >= 0) {
                            val fullPath = cursor.getString(dataIndex)
                            if (!fullPath.isNullOrEmpty()) {
                                // Extract directory path from full path
                                val file = File(fullPath)
                                val parentPath = file.parent
                                if (parentPath != null) {
                                    // Try to make it relative by removing common prefixes
                                    val storagePath = android.os.Environment.getExternalStorageDirectory().absolutePath
                                    return if (parentPath.startsWith(storagePath)) {
                                        parentPath.substring(storagePath.length).removePrefix("/") + "/"
                                    } else {
                                        parentPath + "/"
                                    }
                                }
                            }
                        }
                    }
                }
            } catch (e: Exception) {
                Log.e(TAG, "Error getting relative path from URI: $uri", e)
            }
            return null
        }

        /**
         * Gets the full path with filename for restore purposes.
         * Returns "relative_path/filename" format.
         * Falls back to just filename if path cannot be determined.
         */
        fun getFullPathFromUri(context: Context, uri: Uri): String? {
            val fileName = getDisplayNameFromUri(context, uri)
            if (fileName == null) {
                return null
            }

            val relativePath = getRelativePathFromUri(context, uri)
            return if (relativePath != null) {
                relativePath + fileName
            } else {
                fileName
            }
        }

        /**
         * Gets the last modified timestamp from a URI.
         * Works on all Android versions including Android 10+ with Scoped Storage.
         */
        fun getLastModifiedFromUri(context: Context, uri: Uri): Long? {
            try {
                context.contentResolver.query(uri, null, null, null, null)?.use { cursor ->
                    if (cursor.moveToFirst()) {
                        val modifiedIndex = cursor.getColumnIndex(MediaStore.MediaColumns.DATE_MODIFIED)
                        if (modifiedIndex >= 0) {
                            return cursor.getLong(modifiedIndex)
                        }
                    }
                }
            } catch (e: Exception) {
                Log.e(TAG, "Error getting last modified from URI: $uri", e)
            }
            return null
        }

        /**
         * Gets DATE_TAKEN (capture date) from MediaStore — populated from EXIF by Android.
         * Returns epoch milliseconds, or null if not available.
         * More accurate than DATE_MODIFIED for camera photos and videos.
         */
        fun getDateTakenFromUri(context: Context, uri: Uri): Long? {
            try {
                context.contentResolver.query(uri, null, null, null, null)?.use { cursor ->
                    if (cursor.moveToFirst()) {
                        val takenIndex = cursor.getColumnIndex(MediaStore.MediaColumns.DATE_TAKEN)
                        if (takenIndex >= 0) {
                            val value = cursor.getLong(takenIndex)
                            if (value > 0) return value
                        }
                    }
                }
            } catch (e: Exception) {
                Log.e(TAG, "Error getting date taken from URI: $uri", e)
            }
            return null
        }

        /**
         * Loads location data from EXIF for images on Android 10+.
         * Requires ACCESS_MEDIA_LOCATION permission.
         */
        fun loadLocationDataIfNeeded(@Suppress("UNUSED_PARAMETER") context: Context, uri: Uri): Uri {
            return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                try {
                    // setRequireOriginal() ensures we get the original file with EXIF data intact
                    MediaStore.setRequireOriginal(uri)
                } catch (e: SecurityException) {
                    Log.w(TAG, "ACCESS_MEDIA_LOCATION permission not granted, EXIF location may be redacted", e)
                    uri
                } catch (e: Exception) {
                    Log.e(TAG, "Error loading location data for URI: $uri", e)
                    uri
                }
            } else {
                uri
            }
        }
    }
}