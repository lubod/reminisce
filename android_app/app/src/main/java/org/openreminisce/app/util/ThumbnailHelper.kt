package org.openreminisce.app.util

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.util.Log
import androidx.exifinterface.media.ExifInterface
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import kotlin.math.roundToInt

class ThumbnailHelper {
    companion object {
        private const val TAG = "ThumbnailHelper"

        /**
         * Scales and saves a bitmap as a thumbnail.
         * Common logic extracted from both image and video thumbnail generation.
         */
        private fun processBitmapToThumbnail(
            bitmap: Bitmap,
            thumbnailFile: File,
            maxWidth: Int,
            maxHeight: Int,
            format: Bitmap.CompressFormat = Bitmap.CompressFormat.JPEG,
            quality: Int = 80
        ): File? {
            return try {
                // Calculate scale to fit within max dimensions while preserving aspect ratio
                val scale = calculateScale(bitmap.width, bitmap.height, maxWidth, maxHeight)

                // Create the scaled thumbnail
                val scaledWidth = (bitmap.width * scale).roundToInt()
                val scaledHeight = (bitmap.height * scale).roundToInt()

                val thumbnailBitmap = Bitmap.createScaledBitmap(bitmap, scaledWidth, scaledHeight, true)

                // Save thumbnail to file
                FileOutputStream(thumbnailFile).use { outputStream ->
                    thumbnailBitmap.compress(format, quality, outputStream)
                    outputStream.flush()
                }

                // Recycle bitmaps to free memory
                if (thumbnailBitmap != bitmap) {
                    bitmap.recycle()
                }
                thumbnailBitmap.recycle()

                Log.d(TAG, "Thumbnail generated: ${thumbnailFile.absolutePath}")
                thumbnailFile
            } catch (e: Exception) {
                Log.e(TAG, "Error processing bitmap to thumbnail", e)
                bitmap.recycle()
                null
            }
        }

        /**
         * Generates a thumbnail for an image file
         * @param imageFile The original image file
         * @param thumbnailDir Directory to save the thumbnail
         * @param maxWidth Maximum width of the thumbnail
         * @param maxHeight Maximum height of the thumbnail
         * @return File object of the generated thumbnail, or null if creation failed
         */
        fun generateImageThumbnail(
            imageFile: File,
            thumbnailDir: File,
            maxWidth: Int = 400,
            maxHeight: Int = 400
        ): File? {
            return try {
                // Decode the original image with bounds only (no memory allocation)
                val options = BitmapFactory.Options().apply {
                    inJustDecodeBounds = true
                }
                BitmapFactory.decodeFile(imageFile.absolutePath, options)

                // Calculate the sample size to reduce memory usage
                options.inSampleSize = calculateInSampleSize(options, maxWidth, maxHeight)

                // Decode the image with the calculated sample size
                options.inJustDecodeBounds = false
                val bitmap = BitmapFactory.decodeFile(imageFile.absolutePath, options)

                if (bitmap == null) {
                    Log.e(TAG, "Failed to decode bitmap for thumbnail: ${imageFile.absolutePath}")
                    return null
                }

                // Determine format based on original file extension
                val format = when {
                    imageFile.name.lowercase().endsWith(".png") -> Bitmap.CompressFormat.PNG
                    else -> Bitmap.CompressFormat.JPEG
                }

                val quality = if (format == Bitmap.CompressFormat.JPEG) 80 else 100

                // Save thumbnail to file
                val thumbnailFile = File(thumbnailDir, "thumb_${imageFile.name}")

                processBitmapToThumbnail(bitmap, thumbnailFile, maxWidth, maxHeight, format, quality)
            } catch (e: Exception) {
                Log.e(TAG, "Error generating image thumbnail: ${imageFile.absolutePath}", e)
                null
            }
        }

        /**
         * Generates a thumbnail for a video file (first frame)
         * @param videoFile The original video file
         * @param thumbnailDir Directory to save the thumbnail
         * @param maxWidth Maximum width of the thumbnail
         * @param maxHeight Maximum height of the thumbnail
         * @return File object of the generated thumbnail, or null if creation failed
         */
        fun generateVideoThumbnail(
            videoFile: File,
            thumbnailDir: File,
            maxWidth: Int = 400,
            maxHeight: Int = 400
        ): File? {
            return try {
                val mediaMetadataRetriever = MediaMetadataRetriever()
                mediaMetadataRetriever.setDataSource(videoFile.absolutePath)

                // Get the first frame of the video as a bitmap
                val bitmap = mediaMetadataRetriever.getFrameAtTime(0, MediaMetadataRetriever.OPTION_CLOSEST_SYNC)
                mediaMetadataRetriever.release()

                if (bitmap == null) {
                    Log.e(TAG, "Failed to get first frame for thumbnail: ${videoFile.absolutePath}")
                    return null
                }

                // Save thumbnail to file
                val thumbnailFile = File(thumbnailDir, "thumb_${videoFile.nameWithoutExtension}.jpg")

                processBitmapToThumbnail(bitmap, thumbnailFile, maxWidth, maxHeight)
            } catch (e: Exception) {
                Log.e(TAG, "Error generating video thumbnail: ${videoFile.absolutePath}", e)
                null
            }
        }
        
        /**
         * Generates a thumbnail for an image from URI (Android 10+ compatible).
         * This method preserves EXIF data including GPS location.
         */
        fun generateImageThumbnailFromUri(
            context: Context,
            uri: Uri,
            fileName: String,
            thumbnailDir: File,
            maxWidth: Int = 400,
            maxHeight: Int = 400
        ): File? {
            return try {
                // Load location data if needed (Android 10+)
                val originalUri = MediaHelper.loadLocationDataIfNeeded(context, uri)

                // Log EXIF data for debugging
                logExifData(context, originalUri, fileName)

                // Open input stream from URI
                val inputStream = context.contentResolver.openInputStream(originalUri)
                    ?: run {
                        Log.e(TAG, "Failed to open input stream for URI: $uri")
                        return null
                    }

                // Decode the original image with bounds only (no memory allocation)
                val options = BitmapFactory.Options().apply {
                    inJustDecodeBounds = true
                }
                BitmapFactory.decodeStream(inputStream, null, options)
                inputStream.close()

                // Calculate the sample size to reduce memory usage
                options.inSampleSize = calculateInSampleSize(options, maxWidth, maxHeight)

                // Decode the image with the calculated sample size
                val inputStream2 = context.contentResolver.openInputStream(originalUri)
                    ?: run {
                        Log.e(TAG, "Failed to open input stream for decoding: $uri")
                        return null
                    }

                options.inJustDecodeBounds = false
                val rawBitmap = BitmapFactory.decodeStream(inputStream2, null, options)
                inputStream2.close()

                if (rawBitmap == null) {
                    Log.e(TAG, "Failed to decode bitmap for thumbnail from URI: $uri")
                    return null
                }

                // Apply EXIF orientation — BitmapFactory.decodeStream does not auto-rotate.
                val orientedBitmap = context.contentResolver.openInputStream(originalUri)?.use { exifStream ->
                    val exif = ExifInterface(exifStream)
                    val rotation = when (exif.getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL)) {
                        ExifInterface.ORIENTATION_ROTATE_90  -> 90f
                        ExifInterface.ORIENTATION_ROTATE_180 -> 180f
                        ExifInterface.ORIENTATION_ROTATE_270 -> 270f
                        else -> 0f
                    }
                    if (rotation != 0f) {
                        val matrix = android.graphics.Matrix().apply { postRotate(rotation) }
                        val rotated = Bitmap.createBitmap(rawBitmap, 0, 0, rawBitmap.width, rawBitmap.height, matrix, true)
                        rawBitmap.recycle()
                        rotated
                    } else {
                        rawBitmap
                    }
                } ?: rawBitmap

                // Determine format based on file extension
                val format = when {
                    fileName.lowercase().endsWith(".png") -> Bitmap.CompressFormat.PNG
                    else -> Bitmap.CompressFormat.JPEG
                }

                val quality = if (format == Bitmap.CompressFormat.JPEG) 80 else 100

                // Save thumbnail to file
                val thumbnailFile = File(thumbnailDir, "thumb_$fileName")

                processBitmapToThumbnail(orientedBitmap, thumbnailFile, maxWidth, maxHeight, format, quality)
            } catch (e: Exception) {
                Log.e(TAG, "Error generating image thumbnail from URI: $uri", e)
                null
            }
        }

        /**
         * Generates a thumbnail for a video from URI (Android 10+ compatible).
         */
        fun generateVideoThumbnailFromUri(
            context: Context,
            uri: Uri,
            fileName: String,
            thumbnailDir: File,
            maxWidth: Int = 400,
            maxHeight: Int = 400
        ): File? {
            return try {
                // Load location data if needed (Android 10+)
                val originalUri = MediaHelper.loadLocationDataIfNeeded(context, uri)

                val mediaMetadataRetriever = MediaMetadataRetriever()
                mediaMetadataRetriever.setDataSource(context, originalUri)

                // Get the first frame of the video as a bitmap
                val bitmap = mediaMetadataRetriever.getFrameAtTime(0, MediaMetadataRetriever.OPTION_CLOSEST_SYNC)
                mediaMetadataRetriever.release()

                if (bitmap == null) {
                    Log.e(TAG, "Failed to get first frame for thumbnail from URI: $uri")
                    return null
                }

                // Save thumbnail to file
                val fileNameWithoutExt = fileName.substringBeforeLast(".")
                val thumbnailFile = File(thumbnailDir, "thumb_$fileNameWithoutExt.jpg")

                processBitmapToThumbnail(bitmap, thumbnailFile, maxWidth, maxHeight)
            } catch (e: Exception) {
                Log.e(TAG, "Error generating video thumbnail from URI: $uri", e)
                null
            }
        }

        /**
         * Logs EXIF data including GPS information for debugging purposes.
         * This helps verify that GPS data is accessible before upload.
         */
        private fun logExifData(context: Context, uri: Uri, fileName: String) {
            try {
                context.contentResolver.openInputStream(uri)?.use { inputStream ->
                    val exif = ExifInterface(inputStream)

                    val hasGps = exif.latLong != null
                    if (hasGps) {
                        val latLong = exif.latLong
                        Log.i(TAG, "EXIF GPS data found for $fileName: Lat=${latLong!![0]}, Lon=${latLong[1]}")

                        // Log other useful EXIF data
                        val datetime = exif.getAttribute(ExifInterface.TAG_DATETIME)
                        val make = exif.getAttribute(ExifInterface.TAG_MAKE)
                        val model = exif.getAttribute(ExifInterface.TAG_MODEL)

                        Log.d(TAG, "EXIF data for $fileName - DateTime: $datetime, Make: $make, Model: $model")
                    } else {
                        Log.w(TAG, "No GPS data found in EXIF for $fileName")
                    }
                }
            } catch (e: Exception) {
                Log.e(TAG, "Error reading EXIF data for $fileName", e)
            }
        }

        /**
         * Calculates the scale factor to fit an image within the specified dimensions
         * while preserving the aspect ratio
         */
        private fun calculateScale(width: Int, height: Int, maxWidth: Int, maxHeight: Int): Float {
            val scaleWidth = maxWidth.toFloat() / width
            val scaleHeight = maxHeight.toFloat() / height
            return scaleWidth.coerceAtMost(scaleHeight).coerceAtMost(1.0f) // Don't upscale
        }

        /**
         * Calculates the sample size for BitmapFactory to reduce memory usage
         */
        private fun calculateInSampleSize(options: BitmapFactory.Options, reqWidth: Int, reqHeight: Int): Int {
            val (height, width) = options.run { outHeight to outWidth }
            var inSampleSize = 1

            if (height > reqHeight || width > reqWidth) {
                val halfHeight = height / 2
                val halfWidth = width / 2

                while (halfHeight / inSampleSize >= reqHeight && halfWidth / inSampleSize >= reqWidth) {
                    inSampleSize *= 2
                }
            }

            return inSampleSize
        }
    }
}