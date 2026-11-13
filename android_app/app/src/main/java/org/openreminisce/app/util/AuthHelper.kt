package org.openreminisce.app.util

import android.content.Context
import android.content.Intent
import org.openreminisce.app.LoginActivity
import okhttp3.*
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import okhttp3.RequestBody.Companion.toRequestBody
import okio.ByteString.Companion.decodeBase64
import org.json.JSONObject
import java.util.concurrent.TimeUnit

internal interface TokenPairStore {
    fun getAccessToken(): String?
    fun setAccessToken(token: String)
    fun getRefreshToken(): String?
    fun setRefreshToken(token: String)
    fun clearCredentials()
}

private class SecureTokenPairStore(private val context: Context) : TokenPairStore {
    override fun getAccessToken(): String? = SecureStorageHelper.getAccessToken(context)
    override fun setAccessToken(token: String) = SecureStorageHelper.setAccessToken(context, token)
    override fun getRefreshToken(): String? = SecureStorageHelper.getRefreshToken(context)
    override fun setRefreshToken(token: String) = SecureStorageHelper.setRefreshToken(context, token)
    override fun clearCredentials() = SecureStorageHelper.clearCredentials(context)
}

class AuthHelper {
    companion object {
        private const val TAG = "AuthHelper"
        private val refreshLock = Any()

        /**
         * Check whether the server needs first-run admin setup.
         * Endpoint: GET /api/auth/setup-status
         */
        fun checkSetupStatus(context: Context, baseUrl: String): Boolean {
            return try {
                val client = createHttpClient(context, baseUrl).newBuilder()
                    .connectTimeout(10, TimeUnit.SECONDS)
                    .readTimeout(10, TimeUnit.SECONDS)
                    .build()
                val request = Request.Builder()
                    .url("${baseUrl.trimEnd('/')}/api/auth/setup-status")
                    .get()
                    .build()
                client.newCall(request).execute().use { response ->
                    if (response.isSuccessful) {
                        val body = response.body?.string() ?: "{}"
                        JSONObject(body).optBoolean("needs_setup", false)
                    } else false
                }
            } catch (e: Exception) {
                LogCollector.w(TAG, "checkSetupStatus failed: ${e.message}")
                false
            }
        }

        /**
         * Create the first admin account during initial setup.
         * Endpoint: POST /api/auth/setup
         * Returns null on success, or an error message string.
         */
        fun setupAdmin(context: Context, username: String, password: String, baseUrl: String): String? {
            return try {
                val client = createHttpClient(context, baseUrl).newBuilder()
                    .connectTimeout(10, TimeUnit.SECONDS)
                    .readTimeout(10, TimeUnit.SECONDS)
                    .build()
                val json = JSONObject().apply {
                    put("username", username)
                    put("password", password)
                }.toString()
                val body = json.toRequestBody("application/json; charset=utf-8".toMediaTypeOrNull())
                val request = Request.Builder()
                    .url("${baseUrl.trimEnd('/')}/api/auth/setup")
                    .post(body)
                    .addHeader("Content-Type", "application/json")
                    .build()
                client.newCall(request).execute().use { response ->
                    if (response.isSuccessful) null
                    else {
                        val errBody = response.body?.string() ?: ""
                        try {
                            JSONObject(errBody).optString("message", "Setup failed (${response.code})")
                        } catch (_: Exception) {
                            "Setup failed (${response.code})"
                        }
                    }
                }
            } catch (e: Exception) {
                LogCollector.e(TAG, "setupAdmin failed: ${e.message}")
                e.message ?: "Setup failed"
            }
        }

        /**
         * Register a new user with username, email, and password.
         * Endpoint: POST /api/auth/register
         */
        fun registerUser(context: Context, username: String, email: String, password: String, baseUrl: String): Boolean {
            LogCollector.d(TAG, "Registering user at $baseUrl/api/auth/register")

            val client = createHttpClient(context, baseUrl)

            val json = JSONObject().apply {
                put("username", username)
                put("email", email)
                put("password", password)
            }.toString()

            val mediaType = "application/json; charset=utf-8".toMediaTypeOrNull()
            val body = json.toRequestBody(mediaType)

            val request = Request.Builder()
                .url("${baseUrl.trimEnd('/')}/api/auth/register")
                .post(body)
                .addHeader("Content-Type", "application/json")
                .build()

            return try {
                LogCollector.d(TAG, "Sending registration request...")
                client.newCall(request).execute().use { response ->
                    LogCollector.d(TAG, "Registration response received. Code: ${response.code}")
                    if (response.isSuccessful) {
                        LogCollector.d(TAG, "Registration successful") // body redacted (may contain account data)
                        true
                    } else {
                        val errorBody = response.body?.string()
                        LogCollector.e(TAG, "Registration failed: ${response.code} - $errorBody")
                        false
                    }
                }
            } catch (e: Exception) {
                LogCollector.e(TAG, "Error during registration", e)
                false
            }
        }

        /**
         * Login with username and password credentials via HTTP and store access + refresh tokens.
         */
        fun loginWithCredentials(context: Context, username: String, password: String, baseUrl: String): Boolean {
            LogCollector.i(TAG, "Login attempt at $baseUrl") // username redacted

            if (baseUrl.isEmpty()) {
                LogCollector.e(TAG, "No server URL configured")
                return false
            }

            val client = createHttpClient(context, baseUrl).newBuilder()
                .connectTimeout(10, TimeUnit.SECONDS)
                .readTimeout(10, TimeUnit.SECONDS)
                .build()

            val json = JSONObject().apply {
                put("username", username)
                put("password", password)
                put("device_id", getDeviceId(context))
            }.toString()

            val body = json.toRequestBody("application/json; charset=utf-8".toMediaTypeOrNull())
            val request = Request.Builder()
                .url("${baseUrl.trimEnd('/')}/api/auth/user-login")
                .post(body)
                .addHeader("Content-Type", "application/json")
                .build()

            return try {
                client.newCall(request).execute().use { response ->
                    LogCollector.i(TAG, "Login response: ${response.code}")
                    if (response.isSuccessful) {
                        val responseJson = JSONObject(response.body?.string() ?: "{}")
                        val token = responseJson.optString("access_token")
                        val refreshToken = responseJson.optString("refresh_token")
                        if (token.isNotEmpty()) {
                            SecureStorageHelper.setAccessToken(context, token)
                            if (refreshToken.isNotEmpty()) {
                                SecureStorageHelper.setRefreshToken(context, refreshToken)
                            }
                            ThumbnailAuthInterceptor.clearCachedToken()
                            true
                        } else {
                            LogCollector.e(TAG, "Login response missing access_token")
                            false
                        }
                    } else {
                        LogCollector.w(TAG, "Login failed: ${response.code}")
                        false
                    }
                }
            } catch (e: Exception) {
                LogCollector.e(TAG, "Login request failed: ${e.message}")
                false
            }
        }

        private fun createHttpClient(context: Context, baseUrl: String): OkHttpClient {
            val builder = OkHttpClient.Builder()
                .connectTimeout(30, TimeUnit.SECONDS)
                .readTimeout(30, TimeUnit.SECONDS)

            NetworkHelper.configureSsl(builder, baseUrl, context)

            return builder.build()
        }

        /**
         * Returns a currently valid access token, or rotates the session using the stored refresh token.
         */
        fun getValidToken(context: Context): String? {
            val storedToken = SecureStorageHelper.getAccessToken(context)
            if (!storedToken.isNullOrEmpty() && isTokenValid(storedToken)) {
                return storedToken
            }

            return refreshAuthToken(context, failedAccessToken = storedToken)
        }

        /**
         * Refreshes the access token using POST /api/auth/refresh (serialized via refreshLock so
         * rotating refresh tokens are never consumed twice by concurrent requests).
         */
        fun refreshAuthToken(context: Context, failedAccessToken: String? = null): String? {
            val baseUrl = PreferenceHelper.getServerUrl(context)
            if (baseUrl.isEmpty()) {
                return null
            }

            val client = createHttpClient(context, baseUrl).newBuilder()
                .connectTimeout(10, TimeUnit.SECONDS)
                .readTimeout(10, TimeUnit.SECONDS)
                .build()

            return performTokenRefresh(
                client = client,
                baseUrl = baseUrl,
                deviceId = getDeviceId(context),
                tokenStore = SecureTokenPairStore(context),
                failedAccessToken = failedAccessToken,
                onTokenRefreshed = { ThumbnailAuthInterceptor.clearCachedToken() }
            )
        }

        internal fun performTokenRefresh(
            client: OkHttpClient,
            baseUrl: String,
            deviceId: String,
            tokenStore: TokenPairStore,
            failedAccessToken: String? = null,
            onTokenRefreshed: () -> Unit = {}
        ): String? {
            synchronized(refreshLock) {
                // If another thread already rotated the token since our check/401, reuse it.
                val currentToken = tokenStore.getAccessToken()
                if (!currentToken.isNullOrEmpty() &&
                    currentToken != failedAccessToken &&
                    isTokenValid(currentToken)
                ) {
                    return currentToken
                }

                val refreshToken = tokenStore.getRefreshToken()
                if (refreshToken.isNullOrEmpty() || baseUrl.isEmpty()) {
                    return null
                }

                val json = JSONObject().apply {
                    put("refresh_token", refreshToken)
                    put("device_id", deviceId)
                }.toString()

                val body = json.toRequestBody("application/json; charset=utf-8".toMediaTypeOrNull())
                val request = Request.Builder()
                    .url("${baseUrl.trimEnd('/')}/api/auth/refresh")
                    .post(body)
                    .addHeader("Content-Type", "application/json")
                    .build()

                return try {
                    client.newCall(request).execute().use { response ->
                        when {
                            response.isSuccessful -> {
                                val responseJson = JSONObject(response.body?.string() ?: "{}")
                                val newToken = responseJson.optString("access_token")
                                val newRefreshToken = responseJson.optString("refresh_token")
                                if (newToken.isNotEmpty()) {
                                    tokenStore.setAccessToken(newToken)
                                    if (newRefreshToken.isNotEmpty()) {
                                        tokenStore.setRefreshToken(newRefreshToken)
                                    }
                                    onTokenRefreshed()
                                    newToken
                                } else {
                                    null
                                }
                            }
                            response.code == 401 || response.code == 403 -> {
                                // Refresh token was rejected/revoked/expired; clear session cleanly.
                                tokenStore.clearCredentials()
                                onTokenRefreshed()
                                null
                            }
                            else -> {
                                // Server 5xx or transient error; preserve tokens for subsequent retry.
                                null
                            }
                        }
                    }
                } catch (_: Exception) {
                    // Network/TLS exception; preserve tokens so caller can retry or surface cert prompt.
                    null
                }
            }
        }

        /**
         * Revokes the current refresh token on the server (best-effort) and clears local credentials.
         */
        fun logoutFromServer(context: Context) {
            val refreshToken = SecureStorageHelper.getRefreshToken(context)
            val baseUrl = PreferenceHelper.getServerUrl(context)
            SecureStorageHelper.clearCredentials(context)
            ThumbnailAuthInterceptor.clearCachedToken()
            if (!refreshToken.isNullOrEmpty() && baseUrl.isNotEmpty()) {
                revokeRefreshTokenOnServer(context.applicationContext, baseUrl, refreshToken)
            }
        }

        fun revokeRefreshTokenOnServer(context: Context, baseUrl: String, refreshToken: String) {
            if (refreshToken.isEmpty() || baseUrl.isEmpty()) return
            try {
                val client = createHttpClient(context, baseUrl).newBuilder()
                    .connectTimeout(5, TimeUnit.SECONDS)
                    .readTimeout(5, TimeUnit.SECONDS)
                    .build()
                val json = JSONObject().apply {
                    put("refresh_token", refreshToken)
                }.toString()
                val body = json.toRequestBody("application/json; charset=utf-8".toMediaTypeOrNull())
                val request = Request.Builder()
                    .url("${baseUrl.trimEnd('/')}/api/auth/logout")
                    .post(body)
                    .addHeader("Content-Type", "application/json")
                    .build()
                client.newCall(request).execute().close()
            } catch (e: Exception) {
                LogCollector.w(TAG, "Failed to revoke refresh token on server: ${e.message}")
            }
        }

        fun decodeJwtExpiryMs(token: String): Long {
            val parts = token.split(".")
            if (parts.size != 3 || parts[1].isEmpty()) {
                return 0L
            }
            return try {
                val base64 = buildString {
                    append(parts[1].replace('-', '+').replace('_', '/'))
                    while (length % 4 != 0) append('=')
                }
                val payloadBytes = base64.decodeBase64()?.toByteArray() ?: return 0L
                val json = JSONObject(String(payloadBytes, Charsets.UTF_8))
                val expSeconds = json.optLong("exp", 0L)
                if (expSeconds <= 0L) 0L else expSeconds * 1000L
            } catch (_: Exception) {
                0L
            }
        }

        fun isTokenValid(token: String): Boolean {
            val expMs = decodeJwtExpiryMs(token)
            if (expMs <= 0L) {
                return false
            }
            return expMs > System.currentTimeMillis()
        }

        /**
         * Discover the home server URL from the relay.
         * Calls GET /api/peers/home-server with the current JWT token.
         */
        fun discoverHomeServer(context: Context, relayUrl: String) {
            try {
                val token = SecureStorageHelper.getAccessToken(context)
                if (token.isNullOrEmpty()) {
                    LogCollector.w(TAG, "discoverHomeServer: no token available")
                    return
                }

                val client = createHttpClient(context, relayUrl)
                val url = "${relayUrl.trimEnd('/')}/api/peers/home-server"

                val request = Request.Builder()
                    .url(url)
                    .get()
                    .addHeader("Authorization", "Bearer $token")
                    .build()

                client.newCall(request).execute().use { response ->
                    if (response.isSuccessful) {
                        val body = response.body?.string()
                        val json = JSONObject(body ?: "{}")
                        val homeServerUrl = json.optString("home_server_url", "")

                        if (homeServerUrl.isNotEmpty()) {
                            LogCollector.i(TAG, "Discovered home server: $homeServerUrl")
                        } else {
                            LogCollector.w(TAG, "discoverHomeServer: empty home_server_url in response")
                        }
                    } else {
                        LogCollector.w(TAG, "discoverHomeServer: ${response.code}")
                    }
                }
            } catch (e: Exception) {
                LogCollector.w(TAG, "discoverHomeServer failed: ${e.message}")
            }
        }

        fun pingServer(context: Context, baseUrl: String): Boolean {
            return try {
                val client = createHttpClient(context, baseUrl).newBuilder()
                    .connectTimeout(5, TimeUnit.SECONDS)
                    .readTimeout(5, TimeUnit.SECONDS)
                    .build()
                val request = Request.Builder()
                    .url("${baseUrl.trimEnd('/')}/api/ping")
                    .get()
                    .build()
                client.newCall(request).execute().use { it.isSuccessful }
            } catch (e: Exception) {
                false
            }
        }

        fun getAuthHeaders(context: Context): Map<String, String> {
            val token = getValidToken(context)
            return mapOf(
                "Authorization" to "Bearer ${token ?: ""}"
            )
        }

        fun getDeviceId(context: Context): String {
            return android.provider.Settings.Secure.getString(
                context.contentResolver,
                android.provider.Settings.Secure.ANDROID_ID
            ) ?: "unknown"
        }

        /**
         * Executes an HTTP request with automatic token refresh on 401 responses.
         */
        fun executeWithTokenRefresh(
            context: Context,
            client: OkHttpClient,
            request: Request,
            onCallCreated: ((Call) -> Unit)? = null
        ): Response {
            val call = client.newCall(request)
            onCallCreated?.invoke(call)
            var response = call.execute()

            if (response.code == 401) {
                LogCollector.w(TAG, "Received 401, attempting to refresh token...")
                val failedToken = request.header("Authorization")?.removePrefix("Bearer ")?.trim()
                val newToken = refreshAuthToken(context, failedAccessToken = failedToken)

                if (!newToken.isNullOrEmpty()) {
                    LogCollector.d(TAG, "Successfully refreshed token, retrying request...")
                    response.close()

                    val retryRequest = request.newBuilder()
                        .header("Authorization", "Bearer $newToken")
                        .build()

                    val retryCall = client.newCall(retryRequest)
                    onCallCreated?.invoke(retryCall)
                    response = retryCall.execute()
                    LogCollector.d(TAG, "Retry request completed. Code: ${response.code}")
                } else if (SecureStorageHelper.getRefreshToken(context).isNullOrEmpty()) {
                    LogCollector.e(TAG, "Failed to refresh token (session expired), redirecting to login")
                    redirectToLogin(context)
                }
            }

            return response
        }

        fun redirectToLogin(context: Context) {
            try {
                SecureStorageHelper.clearCredentials(context)
                ThumbnailAuthInterceptor.clearCachedToken()
                val intent = Intent(context, LoginActivity::class.java).apply {
                    flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK
                    putExtra("session_expired", true)
                }
                context.startActivity(intent)
            } catch (e: Exception) {
                LogCollector.e(TAG, "Failed to redirect to login: ${e.message}")
            }
        }
    }
}