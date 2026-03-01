package org.openreminisce.app.repository

import android.content.Context
import android.util.Log
import org.openreminisce.app.model.ThumbnailInfo
import org.openreminisce.app.model.ThumbnailResponse
import org.openreminisce.app.util.AuthHelper
import org.openreminisce.app.util.AuthenticatedHttpClient
import org.openreminisce.app.util.PreferenceHelper
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.Request
import org.json.JSONObject

class RemoteMediaRepository(private val context: Context) {

    companion object {
        private const val TAG = "RemoteMediaRepository"
        private const val PAGE_LIMIT = 100
    }

    suspend fun fetchRemoteThumbnails(
        page: Int = 1,
        limit: Int = PAGE_LIMIT
    ): Result<ThumbnailResponse> = withContext(Dispatchers.IO) {
        try {
            val serverUrl = PreferenceHelper.getServerUrl(context)
            val token = AuthHelper.getValidToken(context)
            val deviceId = AuthHelper.getDeviceId(context)

            if (serverUrl.isEmpty()) {
                Log.e(TAG, "Server URL is empty")
                return@withContext Result.failure(Exception("Server not configured"))
            }

            if (token == null) {
                Log.e(TAG, "Token is null")
                return@withContext Result.failure(Exception("Not authenticated - please log in again"))
            }

            Log.d(TAG, "Server URL: $serverUrl")
            Log.d(TAG, "Token available: ${token.take(10)}...")
            Log.d(TAG, "Device ID: $deviceId")

            // API endpoint to fetch thumbnails (backend uses /media_thumbnails for both images and videos)
            val apiUrl = "$serverUrl/api/media_thumbnails?page=$page&limit=$limit"
            Log.d(TAG, "Fetching thumbnails from: $apiUrl")

            // Use OkHttp client for better P2P proxy compatibility
            val client = AuthenticatedHttpClient.getClient(context)
            val request = Request.Builder()
                .url(apiUrl)
                .get()
                .addHeader("Authorization", "Bearer $token")
                .addHeader("Accept", "application/json")
                .build()

            Log.d(TAG, "Executing request...")
            val response = client.newCall(request).execute()
            Log.d(TAG, "Response code: ${response.code}")

            if (response.isSuccessful) {
                val responseBody = response.body?.string()
                if (responseBody != null) {
                    val thumbnailResponse = parseThumbnailResponse(responseBody)
                    Log.d(TAG, "Fetched ${thumbnailResponse.thumbnails.size} thumbnails (page $page)")
                    Result.success(thumbnailResponse)
                } else {
                    Log.e(TAG, "Empty response body")
                    Result.failure(Exception("Empty response body"))
                }
            } else {
                val errorBody = response.body?.string() ?: "No error body"
                Log.e(TAG, "HTTP ${response.code}: $errorBody")
                Result.failure(Exception("HTTP ${response.code}: $errorBody"))
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error fetching remote thumbnails", e)
            Result.failure(e)
        }
    }

    private fun parseThumbnailResponse(json: String): ThumbnailResponse {
        val jsonObject = JSONObject(json)
        val thumbnailsArray = jsonObject.getJSONArray("thumbnails")
        val thumbnails = mutableListOf<ThumbnailInfo>()

        for (i in 0 until thumbnailsArray.length()) {
            val item = thumbnailsArray.getJSONObject(i)
            thumbnails.add(
                ThumbnailInfo(
                    hash = item.getString("hash"),
                    created_at = item.getString("created_at"),
                    place = item.optString("place").takeIf { it.isNotEmpty() },
                    mediaType = item.optString("media_type", "image")
                )
            )
        }

        return ThumbnailResponse(
            thumbnails = thumbnails,
            total = jsonObject.getInt("total"),
            page = jsonObject.getInt("page"),
            limit = jsonObject.getInt("limit")
        )
    }
}
