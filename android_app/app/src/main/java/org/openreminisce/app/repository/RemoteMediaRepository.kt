package org.openreminisce.app.repository

import android.content.Context
import android.util.Base64
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONObject
import org.openreminisce.app.model.EnhanceResult
import org.openreminisce.app.model.ImageMetadata
import org.openreminisce.app.model.LocationResult
import org.openreminisce.app.model.MapPoint
import org.openreminisce.app.model.MapPointsResponse
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

        private fun parseThumbnailItem(item: JSONObject, includeSimilarity: Boolean): ThumbnailInfo =
            ThumbnailInfo(
                hash = item.getString("hash"),
                created_at = item.optString("created_at", ""),
                place = item.optString("place").takeIf { it.isNotEmpty() && it != "null" },
                mediaType = item.optString("media_type", "image"),
                name = item.optString("name", ""),
                starred = item.optBoolean("starred", false),
                device_id = item.optString("device_id").takeIf { it.isNotEmpty() && it != "null" },
                distance_km = if (item.has("distance_km") && !item.isNull("distance_km")) {
                    item.optDouble("distance_km").toFloat()
                } else null,
                similarity = if (includeSimilarity) item.optDouble("similarity", 0.0).toFloat() else null,
                thumbnailUrl = item.optString("thumbnail_url").takeIf { it.isNotEmpty() && it != "null" },
                fileSizeBytes = if (item.has("file_size_bytes") && !item.isNull("file_size_bytes")) {
                    item.optLong("file_size_bytes")
                } else null,
                aestheticScore = if (item.has("aesthetic_score") && !item.isNull("aesthetic_score")) {
                    item.optDouble("aesthetic_score").toFloat()
                } else null,
                orientation = if (item.has("orientation") && !item.isNull("orientation")) {
                    item.optInt("orientation")
                } else null
            )

        internal fun parseThumbnailResponse(json: String): ThumbnailResponse {
            val jsonObject = JSONObject(json)
            val thumbnailsArray = jsonObject.getJSONArray("thumbnails")
            val thumbnails = mutableListOf<ThumbnailInfo>()

            for (i in 0 until thumbnailsArray.length()) {
                thumbnails.add(parseThumbnailItem(thumbnailsArray.getJSONObject(i), includeSimilarity = false))
            }

            return ThumbnailResponse(
                thumbnails = thumbnails,
                total = jsonObject.getInt("total"),
                page = jsonObject.getInt("page"),
                limit = jsonObject.getInt("limit")
            )
        }

        internal fun parseSearchResponse(json: String): ThumbnailResponse {
            val jsonObject = JSONObject(json)
            val resultsArray = jsonObject.optJSONArray("results") ?: JSONArray()
            val thumbnails = mutableListOf<ThumbnailInfo>()

            for (i in 0 until resultsArray.length()) {
                thumbnails.add(parseThumbnailItem(resultsArray.getJSONObject(i), includeSimilarity = true))
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

        internal fun parseDeviceIdsResponse(body: String): List<String> {
            val trimmed = body.trim()
            val jsonArray = if (trimmed.startsWith("{")) {
                JSONObject(trimmed).optJSONArray("device_ids") ?: JSONArray()
            } else {
                JSONArray(trimmed)
            }
            val ids = mutableListOf<String>()
            for (i in 0 until jsonArray.length()) {
                val id = jsonArray.optString(i).trim()
                if (id.isNotEmpty() && id != "null") {
                    ids.add(id)
                }
            }
            return ids
        }

        internal fun parsePlacesResponse(body: String): List<LocationResult> {
            val trimmed = body.trim()
            val jsonArray = if (trimmed.startsWith("[")) {
                JSONArray(trimmed)
            } else {
                JSONObject(trimmed).optJSONArray("places") ?: JSONArray()
            }
            return (0 until jsonArray.length()).mapNotNull { i ->
                val item = jsonArray.optJSONObject(i) ?: return@mapNotNull null
                val lat = when {
                    item.has("latitude") && !item.isNull("latitude") -> item.optDouble("latitude", Double.NaN)
                    item.has("lat") && !item.isNull("lat") -> item.optDouble("lat", Double.NaN)
                    else -> Double.NaN
                }
                val lon = when {
                    item.has("longitude") && !item.isNull("longitude") -> item.optDouble("longitude", Double.NaN)
                    item.has("lon") && !item.isNull("lon") -> item.optDouble("lon", Double.NaN)
                    else -> Double.NaN
                }
                if (lat.isNaN() || lon.isNaN() || lat !in -90.0..90.0 || lon !in -180.0..180.0) {
                    return@mapNotNull null
                }
                val name = item.optString("name", "")
                val displayName = item.optString("display_name")
                    .takeIf { it.isNotEmpty() && it != "null" }
                    ?: name
                LocationResult(
                    name = name,
                    latitude = lat,
                    longitude = lon,
                    admin_level = item.optString("admin_level", ""),
                    country_code = item.optString("country_code").takeIf { it.isNotEmpty() && it != "null" },
                    display_name = displayName
                )
            }
        }

        internal fun parseMapPointsResponse(body: String): MapPointsResponse {
            val json = JSONObject(body)
            val pointsArray = json.optJSONArray("points") ?: JSONArray()
            val points = (0 until pointsArray.length()).mapNotNull { i ->
                val item = pointsArray.optJSONObject(i) ?: return@mapNotNull null
                val hash = item.optString("hash", "")
                val lon = item.optDouble("lon", Double.NaN)
                val lat = item.optDouble("lat", Double.NaN)
                if (hash.isEmpty() || lon.isNaN() || lat.isNaN()) return@mapNotNull null
                MapPoint(
                    hash = hash,
                    lon = lon,
                    lat = lat,
                    created_at = item.optString("created_at", ""),
                    place = item.optString("place").takeIf { it.isNotEmpty() && it != "null" },
                    starred = item.optBoolean("starred", false),
                    device_id = item.optString("device_id").takeIf { it.isNotEmpty() && it != "null" },
                    has_thumbnail = item.optBoolean("has_thumbnail", true)
                )
            }
            val total = json.optInt("total", points.size)
            return MapPointsResponse(points = points, total = total)
        }

        internal fun parseUpdatePlaceResponse(body: String): String? =
            JSONObject(body).optString("place").takeIf { it.isNotEmpty() && it != "null" }
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

            val endpointPath = when (filter.mediaType) {
                MediaTypeFilter.IMAGE -> "/api/image_thumbnails"
                MediaTypeFilter.VIDEO -> "/api/video_thumbnails"
                MediaTypeFilter.ALL -> "/api/media_thumbnails"
            }
            val urlBuilder = StringBuilder("${serverUrl.trimEnd('/')}$endpointPath?page=$page&limit=$limit")

            if (filter.starredOnly) urlBuilder.append("&starred_only=true")
            filter.startDate?.let { urlBuilder.append("&start_date=${formatDate(it)}") }
            filter.endDate?.let { urlBuilder.append("&end_date=${formatDate(it)}") }
            filter.labelId?.let { urlBuilder.append("&label_id=$it") }
            filter.deviceId?.takeIf { it.isNotEmpty() && it != "null" }?.let {
                urlBuilder.append("&device_id=${java.net.URLEncoder.encode(it, "UTF-8")}")
            }
            filter.locationLat?.let { lat ->
                filter.locationLon?.let { lon ->
                    urlBuilder.append("&location_lat=$lat&location_lon=$lon&location_radius_km=${filter.locationRadiusKm}")
                }
            }
            if (filter.sortBy != "date") urlBuilder.append("&sort_by=${filter.sortBy}")
            if (filter.sortOrder == "asc") urlBuilder.append("&sort_order=asc")

            val apiUrl = urlBuilder.toString()
            Log.d(TAG, "Fetching thumbnails from: $apiUrl")

            val client = AuthenticatedHttpClient.getClient(context)
            val request = Request.Builder()
                .url(apiUrl)
                .get()
                .addHeader("Authorization", "Bearer $token")
                .addHeader("Accept", "application/json")
                .build()

            val response = AuthHelper.executeWithTokenRefresh(context, client, request)
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

            val urlBuilder = StringBuilder("${serverUrl.trimEnd('/')}/api/search/images")
            urlBuilder.append("?query=${java.net.URLEncoder.encode(query, "UTF-8")}")
            urlBuilder.append("&offset=$offset&limit=$limit")
            urlBuilder.append("&mode=$searchMode")
            urlBuilder.append("&min_similarity=${filter.minSimilarity}")
            val searchMediaType = when (filter.mediaType) {
                MediaTypeFilter.IMAGE -> "image"
                MediaTypeFilter.VIDEO -> "video"
                MediaTypeFilter.ALL -> "all"
            }
            urlBuilder.append("&media_type=$searchMediaType")
            filter.labelId?.let { urlBuilder.append("&label_id=$it") }

            if (filter.starredOnly) urlBuilder.append("&starred_only=true")
            filter.startDate?.let { urlBuilder.append("&start_date=${formatDate(it)}") }
            filter.endDate?.let { urlBuilder.append("&end_date=${formatDate(it)}") }
            filter.deviceId?.takeIf { it.isNotEmpty() && it != "null" }?.let {
                urlBuilder.append("&device_id=${java.net.URLEncoder.encode(it, "UTF-8")}")
            }
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

            val response = AuthHelper.executeWithTokenRefresh(context, client, request)

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

            val response = AuthHelper.executeWithTokenRefresh(context, client, request)
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
                    device_id = json.optString("device_id").takeIf { it.isNotEmpty() && it != "null" },
                    file_size_bytes = if (json.has("file_size_bytes") && !json.isNull("file_size_bytes")) json.optLong("file_size_bytes") else null,
                    width = if (json.has("width") && !json.isNull("width")) json.optInt("width") else null,
                    height = if (json.has("height") && !json.isNull("height")) json.optInt("height") else null,
                    orientation = if (json.has("orientation") && !json.isNull("orientation")) json.optInt("orientation") else null,
                    orientation_label = json.optString("orientation_label").takeIf { it.isNotEmpty() && it != "null" },
                    resolution_label = json.optString("resolution_label").takeIf { it.isNotEmpty() && it != "null" },
                    media_type = json.optString("media_type").takeIf { it.isNotEmpty() && it != "null" }
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

            val response = AuthHelper.executeWithTokenRefresh(context, client, request)
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

            val response = AuthHelper.executeWithTokenRefresh(context, client, request)
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

            val response = AuthHelper.executeWithTokenRefresh(context, client, request)
            if (response.isSuccessful) {
                val body = response.body?.string() ?: return@withContext Result.failure(Exception("Empty response"))
                Result.success(parseDeviceIdsResponse(body))
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
                .url("${serverUrl.trimEnd('/')}/api/search/places?query=$encodedQuery&limit=20")
                .get()
                .addHeader("Authorization", "Bearer $token")
                .addHeader("Accept", "application/json")
                .build()

            val response = AuthHelper.executeWithTokenRefresh(context, client, request)
            if (response.isSuccessful) {
                val body = response.body?.string() ?: return@withContext Result.failure(Exception("Empty response"))
                Result.success(parsePlacesResponse(body))
            } else {
                Result.failure(Exception("HTTP ${response.code}"))
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error searching places", e)
            Result.failure(e)
        }
    }

    suspend fun rotateImage(hash: String, direction: String): Result<Int> = withContext(Dispatchers.IO) {
        try {
            val serverUrl = PreferenceHelper.getServerUrl(context)
            val token = AuthHelper.getValidToken(context) ?: return@withContext Result.failure(Exception("Not authenticated"))

            val payload = JSONObject().apply { put("rotate", direction) }.toString()
            val client = AuthenticatedHttpClient.getClient(context)
            val request = Request.Builder()
                .url("${serverUrl.trimEnd('/')}/api/image/$hash/orientation")
                .post(payload.toRequestBody("application/json".toMediaTypeOrNull()))
                .addHeader("Authorization", "Bearer $token")
                .addHeader("Accept", "application/json")
                .build()

            val response = AuthHelper.executeWithTokenRefresh(context, client, request)
            if (response.isSuccessful) {
                val body = response.body?.string() ?: return@withContext Result.failure(Exception("Empty response"))
                val orientation = JSONObject(body).optInt("orientation", 1)
                Result.success(orientation)
            } else {
                Result.failure(Exception("HTTP ${response.code}"))
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error rotating image $hash ($direction)", e)
            Result.failure(e)
        }
    }

    suspend fun fetchMapPoints(
        filter: MediaFilter = MediaFilter(),
        page: Int = 1,
        limit: Int = 2000
    ): Result<MapPointsResponse> = withContext(Dispatchers.IO) {
        try {
            val serverUrl = PreferenceHelper.getServerUrl(context)
            val token = AuthHelper.getValidToken(context)
            if (serverUrl.isEmpty()) return@withContext Result.failure(Exception("Server not configured"))
            if (token == null) return@withContext Result.failure(Exception("Not authenticated"))

            val urlBuilder = StringBuilder("${serverUrl.trimEnd('/')}/api/map/media?page=$page&limit=$limit")
            if (filter.starredOnly) urlBuilder.append("&starred_only=true")
            filter.labelId?.let { urlBuilder.append("&label_id=$it") }
            filter.startDate?.let { urlBuilder.append("&start_date=${formatDate(it)}") }
            filter.endDate?.let { urlBuilder.append("&end_date=${formatDate(it)}") }
            filter.deviceId?.takeIf { it.isNotEmpty() && it != "null" }?.let {
                urlBuilder.append("&device_id=${java.net.URLEncoder.encode(it, "UTF-8")}")
            }

            val client = AuthenticatedHttpClient.getClient(context)
            val request = Request.Builder()
                .url(urlBuilder.toString())
                .get()
                .addHeader("Authorization", "Bearer $token")
                .addHeader("Accept", "application/json")
                .build()

            AuthHelper.executeWithTokenRefresh(context, client, request).use { response ->
                if (response.isSuccessful) {
                    val body = response.body?.string() ?: return@withContext Result.failure(Exception("Empty response"))
                    Result.success(parseMapPointsResponse(body))
                } else {
                    Result.failure(Exception("HTTP ${response.code}"))
                }
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error fetching map points", e)
            Result.failure(e)
        }
    }

    suspend fun updateImagePlace(
        hash: String,
        place: String?,
        latitude: Double? = null,
        longitude: Double? = null
    ): Result<String?> = withContext(Dispatchers.IO) {
        try {
            val serverUrl = PreferenceHelper.getServerUrl(context)
            val token = AuthHelper.getValidToken(context)
                ?: return@withContext Result.failure(Exception("Not authenticated"))

            val payload = JSONObject().apply {
                if (place.isNullOrBlank()) {
                    put("place", JSONObject.NULL)
                } else {
                    put("place", place.trim())
                    if (latitude != null && longitude != null) {
                        put("latitude", latitude)
                        put("longitude", longitude)
                    }
                }
            }.toString()

            val client = AuthenticatedHttpClient.getClient(context)
            val request = Request.Builder()
                .url("${serverUrl.trimEnd('/')}/api/image/$hash/place")
                .post(payload.toRequestBody("application/json".toMediaTypeOrNull()))
                .addHeader("Authorization", "Bearer $token")
                .addHeader("Accept", "application/json")
                .build()

            AuthHelper.executeWithTokenRefresh(context, client, request).use { response ->
                if (response.isSuccessful) {
                    val body = response.body?.string() ?: return@withContext Result.failure(Exception("Empty response"))
                    Result.success(parseUpdatePlaceResponse(body))
                } else {
                    val errorBody = response.body?.string() ?: ""
                    Result.failure(Exception("HTTP ${response.code}: $errorBody"))
                }
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error updating place for $hash", e)
            Result.failure(e)
        }
    }

    suspend fun enhanceImage(hash: String, mode: String = "auto"): Result<EnhanceResult> = withContext(Dispatchers.IO) {
        try {
            val serverUrl = PreferenceHelper.getServerUrl(context)
            val token = AuthHelper.getValidToken(context)
                ?: return@withContext Result.failure(Exception("Not authenticated"))

            val encodedMode = java.net.URLEncoder.encode(mode, "UTF-8")
            val client = AuthenticatedHttpClient.getClientWithTimeouts(context, 30, 120)
            val request = Request.Builder()
                .url("${serverUrl.trimEnd('/')}/api/image/$hash/enhance?mode=$encodedMode")
                .post("{}".toRequestBody("application/json".toMediaTypeOrNull()))
                .addHeader("Authorization", "Bearer $token")
                .build()

            AuthHelper.executeWithTokenRefresh(context, client, request).use { response ->
                if (response.isSuccessful) {
                    val bytes = response.body?.bytes()
                    if (bytes == null || bytes.isEmpty()) {
                        return@withContext Result.failure(Exception("Empty enhanced image"))
                    }
                    val opsHeader = response.header("X-Enhance-Operations") ?: ""
                    val ops = opsHeader.split(",").map { it.trim() }.filter { it.isNotEmpty() }
                    Result.success(EnhanceResult(imageBytes = bytes, operations = ops))
                } else {
                    val errorBody = response.body?.string() ?: ""
                    Result.failure(Exception("HTTP ${response.code}: $errorBody"))
                }
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error enhancing image $hash ($mode)", e)
            Result.failure(e)
        }
    }

    suspend fun saveEnhancedImage(hash: String, imageBytes: ByteArray): Result<String> = withContext(Dispatchers.IO) {
        try {
            val serverUrl = PreferenceHelper.getServerUrl(context)
            val token = AuthHelper.getValidToken(context)
                ?: return@withContext Result.failure(Exception("Not authenticated"))

            val b64 = Base64.encodeToString(imageBytes, Base64.NO_WRAP)
            val payload = JSONObject().apply { put("image", b64) }.toString()

            val client = AuthenticatedHttpClient.getClientWithTimeouts(context, 30, 120)
            val request = Request.Builder()
                .url("${serverUrl.trimEnd('/')}/api/image/$hash/save-enhanced")
                .post(payload.toRequestBody("application/json".toMediaTypeOrNull()))
                .addHeader("Authorization", "Bearer $token")
                .addHeader("Accept", "application/json")
                .build()

            AuthHelper.executeWithTokenRefresh(context, client, request).use { response ->
                if (response.isSuccessful) {
                    val body = response.body?.string() ?: return@withContext Result.failure(Exception("Empty response"))
                    val savedHash = JSONObject(body).optString("hash", "")
                    Result.success(savedHash)
                } else {
                    val errorBody = response.body?.string() ?: ""
                    Result.failure(Exception("HTTP ${response.code}: $errorBody"))
                }
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error saving enhanced image for $hash", e)
            Result.failure(e)
        }
    }

    private fun formatDate(timestamp: Long): String =
        SimpleDateFormat("yyyy-MM-dd", Locale.US).format(java.util.Date(timestamp))
}
