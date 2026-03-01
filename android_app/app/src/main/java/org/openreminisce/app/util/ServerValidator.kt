package org.openreminisce.app.util

import android.content.Context
import android.util.Log
import okhttp3.Request
import java.net.SocketTimeoutException
import java.net.UnknownHostException
import java.util.concurrent.TimeUnit

/**
 * Utility class for validating server connectivity.
 */
object ServerValidator {
    private const val TAG = "ServerValidator"
    private const val PING_TIMEOUT_SECONDS = 10L

    /**
     * Result of server ping validation.
     */
    sealed class PingResult {
        data class Success(val message: String) : PingResult()
        data class Error(val message: String, val exception: Exception? = null) : PingResult()
    }

    /**
     * Validates server URL by attempting to ping the /ping endpoint.
     *
     * @param context Application context
     * @param serverUrl The server URL to validate (e.g., "http://192.168.1.55:11111")
     * @return PingResult indicating success or failure
     */
    fun pingServer(context: Context, serverUrl: String): PingResult {
        // Validate URL format
        if (!isValidUrl(serverUrl)) {
            return PingResult.Error("Invalid URL format")
        }

        try {
            // Create a temporary client with short timeout for ping
            val client = AuthenticatedHttpClient.getClientWithTimeouts(
                context = context,
                connectTimeoutSeconds = PING_TIMEOUT_SECONDS,
                readTimeoutSeconds = PING_TIMEOUT_SECONDS
            ).newBuilder()
                .connectTimeout(PING_TIMEOUT_SECONDS, TimeUnit.SECONDS)
                .readTimeout(PING_TIMEOUT_SECONDS, TimeUnit.SECONDS)
                .build()

            // Build ping request
            val pingUrl = "$serverUrl/api/ping"
            Log.d(TAG, "Pinging server at: $pingUrl")

            val request = Request.Builder()
                .url(pingUrl)
                .get()
                .build()

            // Execute request
            val response = client.newCall(request).execute()

            Log.d(TAG, "Ping response code: ${response.code}")

            return when {
                response.isSuccessful -> {
                    val body = response.body?.string() ?: ""
                    Log.d(TAG, "Ping successful. Response: $body")
                    PingResult.Success("Server is reachable (${response.code})")
                }
                response.code == 404 -> {
                    // Server is reachable but /ping endpoint doesn't exist
                    // This is acceptable - server is online
                    Log.d(TAG, "Server is reachable but /ping endpoint not found (404)")
                    PingResult.Success("Server is reachable")
                }
                else -> {
                    Log.w(TAG, "Ping failed with status code: ${response.code}")
                    PingResult.Error("Server returned status ${response.code}")
                }
            }
        } catch (e: SocketTimeoutException) {
            Log.e(TAG, "Ping timeout", e)
            return PingResult.Error("Connection timed out", e)
        } catch (e: UnknownHostException) {
            Log.e(TAG, "Unknown host", e)
            return PingResult.Error("Server not found. Check the URL.", e)
        } catch (e: Exception) {
            Log.e(TAG, "Ping failed", e)
            return PingResult.Error("Connection failed: ${e.message ?: "Unknown error"}", e)
        }
    }

    /**
     * Validates if the URL has a proper format (starts with http:// or https://).
     */
    private fun isValidUrl(url: String): Boolean {
        val trimmedUrl = url.trim()
        if (!trimmedUrl.startsWith("http://") && !trimmedUrl.startsWith("https://")) {
            return false
        }

        return try {
            java.net.URL(trimmedUrl)
            true
        } catch (e: Exception) {
            false
        }
    }
}