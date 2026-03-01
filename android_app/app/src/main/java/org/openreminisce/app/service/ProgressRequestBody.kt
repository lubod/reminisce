package org.openreminisce.app.service

import android.content.Context
import android.net.Uri
import org.openreminisce.app.util.MediaHelper
import okhttp3.MediaType
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import okhttp3.RequestBody
import okio.*
import java.io.File
import java.io.InputStream

class ProgressRequestBody private constructor(
    private val file: File?,
    private val context: Context?,
    private val uri: Uri?,
    private val contentLength: Long,
    private val mimeType: String,
    private val callbacks: UploadCallbacks
) : RequestBody() {

    interface UploadCallbacks {
        fun onProgressUpdate(percentage: Int)
    }

    companion object {
        private const val PROGRESS_UPDATE_INTERVAL_MS = 500L // Throttle to 500ms

        /**
         * Creates a ProgressRequestBody from a File (legacy method).
         */
        @JvmStatic
        fun fromFile(file: File, callbacks: UploadCallbacks): ProgressRequestBody {
            return ProgressRequestBody(
                file = file,
                context = null,
                uri = null,
                contentLength = file.length(),
                mimeType = guessMimeType(file.name),
                callbacks = callbacks
            )
        }

        /**
         * Creates a ProgressRequestBody from a URI (Android 10+ compatible).
         * This method preserves EXIF data including GPS location.
         */
        @JvmStatic
        fun fromUri(context: Context, uri: Uri, fileName: String, callbacks: UploadCallbacks): ProgressRequestBody {
            // Load location data if needed (Android 10+)
            val originalUri = MediaHelper.loadLocationDataIfNeeded(context, uri)

            val fileSize = MediaHelper.getFileSizeFromUri(context, originalUri) ?: 0L
            val mimeType = guessMimeType(fileName)

            return ProgressRequestBody(
                file = null,
                context = context,
                uri = originalUri,
                contentLength = fileSize,
                mimeType = mimeType,
                callbacks = callbacks
            )
        }

        private fun guessMimeType(path: String): String {
            return when {
                path.endsWith(".jpg", true) || path.endsWith(".jpeg", true) -> "image/jpeg"
                path.endsWith(".png", true) -> "image/png"
                path.endsWith(".gif", true) -> "image/gif"
                path.endsWith(".mp4", true) -> "video/mp4"
                path.endsWith(".mov", true) -> "video/quicktime"
                path.endsWith(".avi", true) -> "video/x-msvideo"
                path.endsWith(".mkv", true) -> "video/x-matroska"
                else -> "application/octet-stream"
            }
        }
    }

    // Secondary constructor for backward compatibility with File-based approach
    constructor(file: File, callbacks: UploadCallbacks) : this(
        file = file,
        context = null,
        uri = null,
        contentLength = file.length(),
        mimeType = guessMimeType(file.name),
        callbacks = callbacks
    )

    override fun contentLength(): Long = contentLength

    override fun contentType(): MediaType? = mimeType.toMediaTypeOrNull()

    override fun writeTo(sink: BufferedSink) {
        val source: Source = when {
            file != null -> file.source()
            uri != null && context != null -> {
                val inputStream = context.contentResolver.openInputStream(uri)
                    ?: throw IllegalArgumentException("Cannot open input stream for URI: $uri")
                inputStream.source()
            }
            else -> throw IllegalStateException("Either file or uri must be provided")
        }

        val bufferedSource = source.buffer()

        var totalBytesRead = 0L
        val length = contentLength()
        var bytesRead: Int
        var lastProgressUpdate = 0L
        var lastProgress = -1

        try {
            while (bufferedSource.read(sink.buffer, 8192).also { bytesRead = it.toInt() } != -1L) {
                totalBytesRead += bytesRead
                val progress = if (length > 0) {
                    ((totalBytesRead * 100) / length).toInt()
                } else {
                    0
                }

                // Throttle progress updates to avoid overwhelming WorkManager
                val currentTime = System.currentTimeMillis()
                if (progress != lastProgress && (currentTime - lastProgressUpdate >= PROGRESS_UPDATE_INTERVAL_MS)) {
                    callbacks.onProgressUpdate(progress)
                    lastProgressUpdate = currentTime
                    lastProgress = progress
                }

                sink.flush()
            }

            // Always send a final update at 100% when done
            if (length > 0 && totalBytesRead == length) {
                callbacks.onProgressUpdate(100)
            }
        } finally {
            bufferedSource.close()
        }
    }
}