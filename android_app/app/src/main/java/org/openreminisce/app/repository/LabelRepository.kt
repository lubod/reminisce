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
import org.openreminisce.app.model.Label
import org.openreminisce.app.util.AuthHelper
import org.openreminisce.app.util.AuthenticatedHttpClient
import org.openreminisce.app.util.PreferenceHelper

class LabelRepository(private val context: Context) {

    companion object {
        private const val TAG = "LabelRepository"
    }

    suspend fun fetchLabels(): Result<List<Label>> = withContext(Dispatchers.IO) {
        try {
            val serverUrl = PreferenceHelper.getServerUrl(context)
            val token = AuthHelper.getValidToken(context) ?: return@withContext Result.failure(Exception("Not authenticated"))

            val client = AuthenticatedHttpClient.getClient(context)
            val request = Request.Builder()
                .url("$serverUrl/api/labels")
                .get()
                .addHeader("Authorization", "Bearer $token")
                .addHeader("Accept", "application/json")
                .build()

            val response = client.newCall(request).execute()
            if (response.isSuccessful) {
                val body = response.body?.string() ?: return@withContext Result.failure(Exception("Empty response"))
                Result.success(parseLabels(body))
            } else {
                Result.failure(Exception("HTTP ${response.code}"))
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error fetching labels", e)
            Result.failure(e)
        }
    }

    suspend fun createLabel(name: String, color: String): Result<Label> = withContext(Dispatchers.IO) {
        try {
            val serverUrl = PreferenceHelper.getServerUrl(context)
            val token = AuthHelper.getValidToken(context) ?: return@withContext Result.failure(Exception("Not authenticated"))

            val json = JSONObject().apply {
                put("name", name)
                put("color", color)
            }.toString()

            val client = AuthenticatedHttpClient.getClient(context)
            val request = Request.Builder()
                .url("$serverUrl/api/labels")
                .post(json.toRequestBody("application/json".toMediaTypeOrNull()))
                .addHeader("Authorization", "Bearer $token")
                .addHeader("Accept", "application/json")
                .build()

            val response = client.newCall(request).execute()
            if (response.isSuccessful) {
                val body = response.body?.string() ?: return@withContext Result.failure(Exception("Empty response"))
                Result.success(parseLabel(JSONObject(body)))
            } else {
                Result.failure(Exception("HTTP ${response.code}"))
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error creating label", e)
            Result.failure(e)
        }
    }

    suspend fun deleteLabel(id: Int): Result<Unit> = withContext(Dispatchers.IO) {
        try {
            val serverUrl = PreferenceHelper.getServerUrl(context)
            val token = AuthHelper.getValidToken(context) ?: return@withContext Result.failure(Exception("Not authenticated"))

            val client = AuthenticatedHttpClient.getClient(context)
            val request = Request.Builder()
                .url("$serverUrl/api/labels/$id")
                .delete()
                .addHeader("Authorization", "Bearer $token")
                .build()

            val response = client.newCall(request).execute()
            if (response.isSuccessful) Result.success(Unit)
            else Result.failure(Exception("HTTP ${response.code}"))
        } catch (e: Exception) {
            Log.e(TAG, "Error deleting label $id", e)
            Result.failure(e)
        }
    }

    suspend fun getMediaLabels(hash: String, type: String): Result<List<Label>> = withContext(Dispatchers.IO) {
        try {
            val serverUrl = PreferenceHelper.getServerUrl(context)
            val token = AuthHelper.getValidToken(context) ?: return@withContext Result.failure(Exception("Not authenticated"))
            val endpoint = if (type == "video") "video" else "image"

            val client = AuthenticatedHttpClient.getClient(context)
            val request = Request.Builder()
                .url("$serverUrl/api/$endpoint/$hash/labels")
                .get()
                .addHeader("Authorization", "Bearer $token")
                .addHeader("Accept", "application/json")
                .build()

            val response = client.newCall(request).execute()
            if (response.isSuccessful) {
                val body = response.body?.string() ?: return@withContext Result.failure(Exception("Empty response"))
                Result.success(parseLabels(body))
            } else {
                Result.failure(Exception("HTTP ${response.code}"))
            }
        } catch (e: Exception) {
            Log.e(TAG, "Error fetching labels for $hash", e)
            Result.failure(e)
        }
    }

    suspend fun addLabelToMedia(hash: String, type: String, labelId: Int): Result<Unit> = withContext(Dispatchers.IO) {
        try {
            val serverUrl = PreferenceHelper.getServerUrl(context)
            val token = AuthHelper.getValidToken(context) ?: return@withContext Result.failure(Exception("Not authenticated"))
            val endpoint = if (type == "video") "video" else "image"

            val client = AuthenticatedHttpClient.getClient(context)
            val request = Request.Builder()
                .url("$serverUrl/api/$endpoint/$hash/labels/$labelId")
                .post("{}".toRequestBody("application/json".toMediaTypeOrNull()))
                .addHeader("Authorization", "Bearer $token")
                .build()

            val response = client.newCall(request).execute()
            if (response.isSuccessful) Result.success(Unit)
            else Result.failure(Exception("HTTP ${response.code}"))
        } catch (e: Exception) {
            Log.e(TAG, "Error adding label $labelId to $hash", e)
            Result.failure(e)
        }
    }

    suspend fun removeLabelFromMedia(hash: String, type: String, labelId: Int): Result<Unit> = withContext(Dispatchers.IO) {
        try {
            val serverUrl = PreferenceHelper.getServerUrl(context)
            val token = AuthHelper.getValidToken(context) ?: return@withContext Result.failure(Exception("Not authenticated"))
            val endpoint = if (type == "video") "video" else "image"

            val client = AuthenticatedHttpClient.getClient(context)
            val request = Request.Builder()
                .url("$serverUrl/api/$endpoint/$hash/labels/$labelId")
                .delete()
                .addHeader("Authorization", "Bearer $token")
                .build()

            val response = client.newCall(request).execute()
            if (response.isSuccessful) Result.success(Unit)
            else Result.failure(Exception("HTTP ${response.code}"))
        } catch (e: Exception) {
            Log.e(TAG, "Error removing label $labelId from $hash", e)
            Result.failure(e)
        }
    }

    private fun parseLabels(json: String): List<Label> {
        return try {
            // Try as array first, then as object with "labels" key
            val jsonArray = try {
                JSONArray(json)
            } catch (e: Exception) {
                JSONObject(json).optJSONArray("labels") ?: JSONArray()
            }
            (0 until jsonArray.length()).map { parseLabel(jsonArray.getJSONObject(it)) }
        } catch (e: Exception) {
            Log.e(TAG, "Error parsing labels JSON", e)
            emptyList()
        }
    }

    private fun parseLabel(obj: JSONObject) = Label(
        id = obj.getInt("id"),
        name = obj.getString("name"),
        color = obj.optString("color", "#808080"),
        created_at = obj.optString("created_at", "")
    )
}
