package org.openreminisce.app.util

import android.content.Context
import android.net.Uri
import android.util.Log
import org.openreminisce.app.rust.Blake3Hasher
import java.io.File

class HashCalculator {
    companion object {
        private const val TAG = "HashCalculator"
        private const val PROGRESS_UPDATE_INTERVAL_MS = 500L // Update progress at most every 500ms
        private const val BUFFER_SIZE = 65536 // 64KB buffer for efficient I/O

        /**
         * Calculates BLAKE3 hash from a File object.
         * Legacy method - prefer calculateHashFromUri for Android 10+ compatibility.
         */
        fun calculateHash(file: File, onProgress: ((Float) -> Unit)? = null, shouldCancel: (() -> Boolean)? = null): String {
            val hasher = Blake3Hasher()
            val buffer = ByteArray(BUFFER_SIZE)
            var bytesRead: Int
            val inputStream = file.inputStream()

            val totalSize = file.length()
            var bytesProcessed = 0L
            var lastProgressUpdate = 0L

            try {
                while (inputStream.read(buffer).also { bytesRead = it } != -1) {
                    // Check if we should cancel
                    if (shouldCancel?.invoke() == true) {
                        Log.d(TAG, "Hash calculation cancelled for file: ${file.name}")
                        inputStream.close()
                        throw InterruptedException("Hash calculation cancelled")
                    }

                    // Update hasher with the bytes read (only the valid portion)
                    val chunk = if (bytesRead == buffer.size) buffer else buffer.copyOf(bytesRead)
                    hasher.update(chunk)
                    bytesProcessed += bytesRead

                    // Report progress if callback is provided, but throttle updates
                    val currentTime = System.currentTimeMillis()
                    if (currentTime - lastProgressUpdate >= PROGRESS_UPDATE_INTERVAL_MS) {
                        onProgress?.let {
                            val progress = bytesProcessed.toFloat() / totalSize.toFloat()
                            it(progress)
                        }
                        lastProgressUpdate = currentTime
                    }
                }

                // Send final progress update
                onProgress?.let {
                    it(1.0f)
                }

                inputStream.close()

                return hasher.finalize()
            } catch (e: InterruptedException) {
                inputStream.close()
                throw e
            }
        }

        /**
         * Calculates BLAKE3 hash from a URI using ContentResolver.
         * This method works on all Android versions including Android 10+ with Scoped Storage.
         * @param context Application context
         * @param uri Content URI of the file
         * @param fileSize Size of the file in bytes (for progress reporting)
         * @param onProgress Optional progress callback (0.0 to 1.0)
         * @param shouldCancel Optional cancellation check callback
         * @return BLAKE3 hash as hexadecimal string
         */
        fun calculateHashFromUri(
            context: Context,
            uri: Uri,
            fileSize: Long,
            onProgress: ((Float) -> Unit)? = null,
            shouldCancel: (() -> Boolean)? = null
        ): String {
            val hasher = Blake3Hasher()
            val buffer = ByteArray(BUFFER_SIZE)
            var bytesRead: Int
            var bytesProcessed = 0L
            var lastProgressUpdate = 0L

            // Load location data if needed (Android 10+)
            val originalUri = MediaHelper.loadLocationDataIfNeeded(context, uri)

            val inputStream = context.contentResolver.openInputStream(originalUri)
                ?: throw IllegalArgumentException("Cannot open input stream for URI: $uri")

            try {
                while (inputStream.read(buffer).also { bytesRead = it } != -1) {
                    // Check if we should cancel
                    if (shouldCancel?.invoke() == true) {
                        Log.d(TAG, "Hash calculation cancelled for URI: $uri")
                        inputStream.close()
                        throw InterruptedException("Hash calculation cancelled")
                    }

                    // Update hasher with the bytes read (only the valid portion)
                    val chunk = if (bytesRead == buffer.size) buffer else buffer.copyOf(bytesRead)
                    hasher.update(chunk)
                    bytesProcessed += bytesRead

                    // Report progress if callback is provided, but throttle updates
                    val currentTime = System.currentTimeMillis()
                    if (currentTime - lastProgressUpdate >= PROGRESS_UPDATE_INTERVAL_MS) {
                        onProgress?.let {
                            val progress = if (fileSize > 0) {
                                bytesProcessed.toFloat() / fileSize.toFloat()
                            } else {
                                0f
                            }
                            it(progress)
                        }
                        lastProgressUpdate = currentTime
                    }
                }

                // Send final progress update
                onProgress?.let {
                    it(1.0f)
                }

                inputStream.close()

                return hasher.finalize()
            } catch (e: InterruptedException) {
                inputStream.close()
                throw e
            } catch (e: Exception) {
                inputStream.close()
                throw e
            }
        }
    }
}
