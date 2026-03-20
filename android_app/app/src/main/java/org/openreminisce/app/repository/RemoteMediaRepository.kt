package org.openreminisce.app.repository

import android.content.Context
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONObject
import org.openreminisce.app.model.ImageMetadata
import org.openreminisce.app.model.LocationResult
import org.openreminisce.app.model.MediaFilter
import org.openreminisce.app.model.MediaTypeFilter
import org.openreminisce.app.model.SearchMode
import org.openreminisce.app.model.SearchResponse
import org.openreminisce.app.model.SearchResult
import org.openreminisce.app.model.StarResponse
import org.openreminisce.app.model.ThumbnailInfo
import org.openreminisce.app.model.ThumbnailResponse
import org.openreminisce.app.util.AuthHelper
import org.openreminisce.app.util.AuthenticatedHttpClient
import org.openreminisce.app.util.PreferenceHelper
import java.text.SimpleDateFormat
import java.util.Locale

class RemoteMediaRepository(private val context: Context) {

    companion object {
        private const val TAG = "RemoteMediaRepository"
        private const val PAGE_LIMIT = 50
    }

    suspend fun fetchRemoteThumbnails(
        page: Int = 1,
        limit: Int = PAGE_LIMIT,
        filter: MediaFilter = MediaFilter()
    ): Result<ThumbnailResponse> = withContext(Dispatchers.IO) {
        try {
            val serverUrl = PreferenceHelper.getServerUrl(context)
            val token = AuthHelper.getValidToken(context)

            if (serverUrl.isEmpty()) return@withContext Result.failure(Exception("Server not configured"))
            if (token == null) return@withContext Result.failure(Exception("Not authenticated - please log in again"))

            val urlBuilder = StringBuilder("$serverUrl/api/media_thumbnails?page=$page&limit=$limit")

            when (filter.mediaType) {
                MediaTypeFilter.IMAGE -> urlBuilder.append("&media_type=image")
                MediaTypeFilter.VIDEO -> urlBuilder.append("&media_type=video")
                MediaTypeFilter.ALL -> { /* no param */ }
            }
            if (filter.starredOnly) urlBuilder.append("&starred=true")
            filter.startDate?.let { urlBuilder.append("&start_date=${formatDate(it)}") }
            filter.endDate?.let { urlBuilder.append("&end_date=${formatDate(it)}") }
            filter.labelId?.let { urlBuilder.append("&label_id=$it") }
            filter.deviceId?.takeIf { it.isNotEmpty() && it != "null" }?.let { urlBuilder.append("&device_id=$it") }
            filter.locationLat?.let { lat ->
                filter.locationLon?.let { lon ->
                    urlBuilder.append("&lat=$lat&lon=$lon&radius_km=${filter.locationRadiusKm}")
                }
            }

            val apiUrl = urlBuilder.toString()
            Log.d(TAG, "Fetching thumbnails from: $apiUrl")

            val client = AuthenticatedHttpClient.getClient(context)
            val request = Request.Builder()
                .url(apiUrl)
                .get()
                .addHeader("Authorization", "Bearer $token")
                .addHeader("Accept", "application/json")
                .build()

            val response = client.newCall(request).execute()
            Log.d(TAG, "Response code: ${response.code}")

            if (response.isSuccessful) {
                val responseBody = response.body?.string()
                if (responseBody != null) {
                    val thumbnailResponse = parseThumbnailResponse(responseBody)
                    Log.d(TAG, "Fetched ${thumbnailResponse.thumbnails.size} thumbnails (page $page)")
                    Result.success(thumbnailResponse)
                } else {
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

    suspend fun searchMedia(
        query: String,
        offset: Int = 0,
        limit: Int = PAGE_LIMIT,
        filter: MediaFilter = MediaFilter()
    ): Result<ThumbnailResponse> = withContext(Dispatchers.IO) {
        try {
            val serverUrl = PreferenceHelper.getServerUrl(context)
            val token = AuthHelper.getValidToken(context)

            if (serverUrl.isEmpty()) return@withContext Result.failure(Exception("Server not configured"))
            if (token == null) return@withContext Result.failure(Exception("Not authenticated"))

            val searchMode = when (filter.searchMode) {
                SearchMode.SEMANTIC -> "semantic"
                SearchMode.TEXT -> "text"
                SearchMode.HYBRID -> "hybrid"
            }

            val urlBuilder = StringBuilder("$serverUrl/api/search/images")
            urlBuilder.append("?query=${java.net.URLEncoder.encode(query, "UTF-8")}")
            urlBuilder.append("&offset=$offset&limit=$limit")
            urlBuilder.append("&mode=$searchMode")
            urlBuilder.append("&min_similarity=${filter.minSimilarity}")

            if (filter.starredOnly) urlBuilder.append("&starred_only=true")
            filter.startDate?.let { urlBuilder.append("&start_date=${formatDate(it)}") }
            filter.endDate?.let { urlBuilder.append("&end_date=${formatDate(it)}") }
            filter.deviceId?.takeIf { it.isNotEmpty() && it != "null" }?.let { urlBuilder.append("&device_id=$it") }
            filter.locationLat?.let { lat ->
                filter.locationLon?.let { lon ->
                    urlBuilder.append("&location_lat=$lat&location_lon=$lon&location_radius_km=${filter.locationRadiusKm}")
                }
            }

            val client = AuthenticatedHttpClient.getClient(context)
            val request = Request.Builder()
                .url(urlBuilder.toString())
                .get()
                .addHeader("Authorization", "Bearer $token")
                .addHeader("Accept", "application/json")
                .build()

            val response = client.newCall(request).execute()

            if (response.isSuccessful) {
                val body = response.body?.string() ?: return@withContext Result.failure(Exception("Empty response"))
                val searchResponse = parseSearchResponse(body)
                Result.success(searchResponse)
            } else {
                val errorBody = response.body?.string() ?: "No error body"
                Result.failure(Exception("HTTP ${response.code}: $errorBody"))
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error searching media", e)
            Result.failure(e)
        }
    }

    suspend fun fetchMetadata(hash: String): Result<ImageMetadata> = withContext(Dispatchers.IO) {
        try {
            val serverUrl = PreferenceHelper.getServerUrl(context)
            val token = AuthHelper.getValidToken(context) ?: return@withContext Result.failure(Exception("Not authenticated"))

            val client = AuthenticatedHttpClient.getClient(context)
            val request = Request.Builder()
                .url("$serverUrl/api/image/$hash/metadata")
                .get()
                .addHeader("Authorization", "Bearer $token")
                .addHeader("Accept", "application/json")
                .build()

            val response = client.newCall(request).execute()
            if (response.isSuccessful) {
                val body = response.body?.string() ?: return@withContext Result.failure(Exception("Empty response"))
                val json = JSONObject(body)
                val metadata = ImageMetadata(
                    hash = json.optString("hash", hash),
                    name = json.optString("name", ""),
                    description = json.optString("description").takeIf { it.isNotEmpty() && it != "null" },
                    place = json.optString("place").takeIf { it.isNotEmpty() && it != "null" },
                    created_at = json.optString("created_at", ""),
                    exif = if (json.has("exif") && !json.isNull("exif")) json.get("exif").toString() else null,
                    starred = json.optBoolean("starred", false),
                    device_id = json.optString("device_id").takeIf { it.isNotEmpty() && it != "null" }
                )
                Result.success(metadata)
            } else {
                Result.failure(Exception("HTTP ${response.code}"))
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error fetching metadata for $hash", e)
            Result.failure(e)
        }
    }

    suspend fun toggleStar(hash: String, mediaType: String): Result<StarResponse> = withContext(Dispatchers.IO) {
        try {
            val serverUrl = PreferenceHelper.getServerUrl(context)
            val token = AuthHelper.getValidToken(context) ?: return@withContext Result.failure(Exception("Not authenticated"))

            val endpoint = if (mediaType == "video") "video" else "image"
            val client = AuthenticatedHttpClient.getClient(context)
            val request = Request.Builder()
                .url("$serverUrl/api/$endpoint/$hash/star")
                .post("{}".toRequestBody("application/json".toMediaTypeOrNull()))
                .addHeader("Authorization", "Bearer $token")
                .addHeader("Accept", "application/json")
                .build()

            val response = client.newCall(request).execute()
            if (response.isSuccessful) {
                val body = response.body?.string() ?: return@withContext Result.failure(Exception("Empty response"))
                val json = JSONObject(body)
                Result.success(StarResponse(
                    hash = json.optString("hash", hash),
                    starred = json.optBoolean("starred", false)
                ))
            } else {
                Result.failure(Exception("HTTP ${response.code}"))
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error toggling star for $hash", e)
            Result.failure(e)
        }
    }

    suspend fun deleteMedia(hash: String, mediaType: String): Result<Unit> = withContext(Dispatchers.IO) {
        try {
            val serverUrl = PreferenceHelper.getServerUrl(context)
            val token = AuthHelper.getValidToken(context) ?: return@withContext Result.failure(Exception("Not authenticated"))

            val endpoint = if (mediaType == "video") "video" else "image"
            val client = AuthenticatedHttpClient.getClient(context)
            val request = Request.Builder()
                .url("$serverUrl/api/$endpoint/$hash/delete")
                .post("{}".toRequestBody("application/json".toMediaTypeOrNull()))
                .addHeader("Authorization", "Bearer $token")
                .build()

            val response = client.newCall(request).execute()
            if (response.isSuccessful) {
                Result.success(Unit)
            } else {
                Result.failure(Exception("HTTP ${response.code}"))
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error deleting media $hash", e)
            Result.failure(e)
        }
    }

    suspend fun fetchDeviceIds(): Result<List<String>> = withContext(Dispatchers.IO) {
        try {
            val serverUrl = PreferenceHelper.getServerUrl(context)
            val token = AuthHelper.getValidToken(context) ?: return@withContext Result.failure(Exception("Not authenticated"))

            val client = AuthenticatedHttpClient.getClient(context)
            val request = Request.Builder()
                .url("$serverUrl/api/device_ids")
                .get()
                .addHeader("Authorization", "Bearer $token")
                .addHeader("Accept", "application/json")
                .build()

            val response = client.newCall(request).execute()
            if (response.isSuccessful) {
                val body = response.body?.string() ?: return@withContext Result.failure(Exception("Empty response"))
                val jsonArray = JSONArray(body)
                val ids = (0 until jsonArray.length()).map { jsonArray.getString(it) }
                Result.success(ids)
            } else {
                Result.failure(Exception("HTTP ${response.code}"))
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error fetching device IDs", e)
            Result.failure(e)
        }
    }

    suspend fun searchPlaces(query: String): Result<List<LocationResult>> = withContext(Dispatchers.IO) {
        try {
            val serverUrl = PreferenceHelper.getServerUrl(context)
            val token = AuthHelper.getValidToken(context) ?: return@withContext Result.failure(Exception("Not authenticated"))

            val encodedQuery = java.net.URLEncoder.encode(query, "UTF-8")
            val client = AuthenticatedHttpClient.getClient(context)
            val request = Request.Builder()
                .url("$serverUrl/api/search/places?q=$encodedQuery")
                .get()
                .addHeader("Authorization", "Bearer $token")
                .addHeader("Accept", "application/json")
                .build()

            val response = client.newCall(request).execute()
            if (response.isSuccessful) {
                val body = response.body?.string() ?: return@withContext Result.failure(Exception("Empty response"))
                val jsonArray = JSONArray(body)
                val results = (0 until jsonArray.length()).mapNotNull { i ->
                    val item = jsonArray.getJSONObject(i)
                    val lat = item.optDouble("latitude", Double.NaN)
                    val lon = item.optDouble("longitude", Double.NaN)
                    if (lat.isNaN() || lon.isNaN() || lat !in -90.0..90.0 || lon !in -180.0..180.0) {
                        return@mapNotNull null
                    }
                    LocationResult(
                        name = item.optString("name", ""),
                        latitude = lat,
                        longitude = lon,
                        admin_level = item.optString("admin_level", ""),
                        country_code = item.optString("country_code").takeIf { it.isNotEmpty() && it != "null" },
                        display_name = item.optString("display_name", item.optString("name", ""))
                    )
                }
                Result.success(results)
            } else {
                Result.failure(Exception("HTTP ${response.code}"))
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error searching places", e)
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
                    place = item.optString("place").takeIf { it.isNotEmpty() && it != "null" },
                    mediaType = item.optString("media_type", "image"),
                    name = item.optString("name", ""),
                    starred = item.optBoolean("starred", false),
                    device_id = item.optString("device_id").takeIf { it.isNotEmpty() && it != "null" }
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

    private fun parseSearchResponse(json: String): ThumbnailResponse {
        val jsonObject = JSONObject(json)
        val resultsArray = jsonObject.optJSONArray("results") ?: JSONArray()
        val thumbnails = mutableListOf<ThumbnailInfo>()

        for (i in 0 until resultsArray.length()) {
            val item = resultsArray.getJSONObject(i)
            thumbnails.add(
                ThumbnailInfo(
                    hash = item.getString("hash"),
                    created_at = item.optString("created_at", ""),
                    place = item.optString("place").takeIf { it.isNotEmpty() && it != "null" },
                    mediaType = item.optString("media_type", "image"),
                    name = item.optString("name", ""),
                    starred = item.optBoolean("starred", false),
                    device_id = item.optString("device_id").takeIf { it.isNotEmpty() && it != "null" },
                    similarity = item.optDouble("similarity", 0.0).toFloat()
                )
            )
        }

        val total = jsonObject.optInt("total", thumbnails.size)
        // Search responses use offset-based pagination; page field defaults to 1
        return ThumbnailResponse(
            thumbnails = thumbnails,
            total = total,
            page = 1,
            limit = thumbnails.size
        )
    }

    private val dateFormatter = SimpleDateFormat("yyyy-MM-dd", Locale.US)

    private fun formatDate(timestamp: Long): String =
        dateFormatter.format(java.util.Date(timestamp))
}
