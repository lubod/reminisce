package org.openreminisce.app.util

import android.content.Context
import android.content.Intent
import org.openreminisce.app.LoginActivity
import okhttp3.*
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONObject
import java.util.concurrent.TimeUnit
import javax.net.ssl.*
import java.security.cert.X509Certificate



class AuthHelper {
    companion object {
        private const val TAG = "AuthHelper"



        /**
         * Check whether the server needs first-run admin setup.
         * Endpoint: GET /auth/setup-status
         */
        fun checkSetupStatus(baseUrl: String): Boolean {
            return try {
                val client = createHttpClient(baseUrl).newBuilder()
                    .connectTimeout(10, TimeUnit.SECONDS)
                    .readTimeout(10, TimeUnit.SECONDS)
                    .build()
                val request = Request.Builder()
                    .url("${baseUrl.trimEnd('/')}/api/auth/setup-status")
                    .get()
                    .build()
                val response = client.newCall(request).execute()
                if (response.isSuccessful) {
                    val body = response.body?.string() ?: "{}"
                    JSONObject(body).optBoolean("needs_setup", false)
                } else false
            } catch (e: Exception) {
                LogCollector.w(TAG, "checkSetupStatus failed: ${e.message}")
                false
            }
        }

        /**
         * Create the first admin account during initial setup.
         * Endpoint: POST /auth/setup
         * Returns null on success, or an error message string.
         */
        fun setupAdmin(username: String, password: String, baseUrl: String): String? {
            return try {
                val client = createHttpClient(baseUrl).newBuilder()
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
                val response = client.newCall(request).execute()
                if (response.isSuccessful) null
                else {
                    val errBody = response.body?.string() ?: ""
                    JSONObject(errBody).optString("message", "Setup failed (${response.code})")
                }
            } catch (e: Exception) {
                LogCollector.e(TAG, "setupAdmin failed: ${e.message}")
                e.message ?: "Setup failed"
            }
        }

        /**
         * Register a new user with username, email, and password.
         * Endpoint: POST /auth/register
         */
        fun registerUser(username: String, email: String, password: String, baseUrl: String): Boolean {
            LogCollector.d(TAG, "Registering user: $username at $baseUrl/api/auth/register")

            val client = createHttpClient(baseUrl)

            val json = JSONObject().apply {
                put("username", username)
                put("email", email)
                put("password", password)
            }.toString()

            val mediaType = "application/json; charset=utf-8".toMediaTypeOrNull()
            val body = json.toRequestBody(mediaType)

            val request = Request.Builder()
                .url("$baseUrl/api/auth/register")
                .post(body)
                .addHeader("Content-Type", "application/json")
                .build()

            return try {
                LogCollector.d(TAG, "Sending registration request...")
                val response = client.newCall(request).execute()

                LogCollector.d(TAG, "Registration response received. Code: ${response.code}")

                if (response.isSuccessful) {
                    val responseBody = response.body?.string()
                    LogCollector.d(TAG, "Registration successful: $responseBody")
                    true
                } else {
                    val errorBody = response.body?.string()
                    LogCollector.e(TAG, "Registration failed: ${response.code} - $errorBody")
                    false
                }
            } catch (e: Exception) {
                LogCollector.e(TAG, "Error during registration", e)
                false
            }
        }

        /**
         * Login with username and password credentials via HTTP.
         */
        fun loginWithCredentials(context: Context, username: String, password: String, baseUrl: String): Boolean {
            LogCollector.i(TAG, "Login attempt for user: $username at $baseUrl")

            if (baseUrl.isEmpty()) {
                LogCollector.e(TAG, "No server URL configured")
                return false
            }

            val client = createHttpClient(baseUrl).newBuilder()
                .connectTimeout(10, TimeUnit.SECONDS)
                .readTimeout(10, TimeUnit.SECONDS)
                .build()

            val json = JSONObject().apply {
                put("username", username)
                put("password", password)
            }.toString()

            val body = json.toRequestBody("application/json; charset=utf-8".toMediaTypeOrNull())
            val request = Request.Builder()
                .url("${baseUrl.trimEnd('/')}/api/auth/user-login")
                .post(body)
                .addHeader("Content-Type", "application/json")
                .build()

            return try {
                val response = client.newCall(request).execute()
                LogCollector.i(TAG, "Login response: ${response.code}")

                if (response.isSuccessful) {
                    val token = JSONObject(response.body?.string() ?: "").optString("access_token")
                    if (token.isNotEmpty()) {
                        SecureStorageHelper.setAccessToken(context, token)
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
            } catch (e: Exception) {
                LogCollector.e(TAG, "Login request failed: ${e.message}")
                false
            }
        }

        private fun createHttpClient(baseUrl: String): OkHttpClient {
            val useInsecure = NetworkHelper.isPrivateOrLocalAddress(baseUrl)
            LogCollector.d(TAG, "HTTP client: ${if (useInsecure) "insecure" else "secure"} for $baseUrl")

            val builder = OkHttpClient.Builder()
                .connectTimeout(30, TimeUnit.SECONDS)
                .readTimeout(30, TimeUnit.SECONDS)

            NetworkHelper.configureInsecureSsl(builder, baseUrl)

            return builder.build()
        }

        /**
         * Get valid token - tries to use stored token if valid, otherwise re-authenticates.
         * Supports both new credential-based and legacy API secret authentication.
         */
        fun getValidToken(context: Context): String? {
            LogCollector.d(TAG, "Checking for valid token...")

            // Check if the stored token is valid first
            val storedToken = SecureStorageHelper.getAccessToken(context)
            LogCollector.d(TAG, "Retrieved stored token, length: ${storedToken?.length ?: 0}")

            if (storedToken != null && isTokenValid(storedToken)) {
                LogCollector.d(TAG, "Stored token is still valid")
                return storedToken
            }

            LogCollector.d(TAG, "Stored token is invalid or doesn't exist, attempting to login")

            // Try new credential-based authentication first
            val username = SecureStorageHelper.getUsername(context)
            val password = SecureStorageHelper.getPassword(context)

            if (!username.isNullOrEmpty() && !password.isNullOrEmpty()) {
                val baseUrl = PreferenceHelper.getServerUrl(context)
                LogCollector.d(TAG, "Using credential-based authentication")

                if (loginWithCredentials(context, username, password, baseUrl)) {
                    LogCollector.d(TAG, "Login successful, retrieving new token")
                    return SecureStorageHelper.getAccessToken(context)
                } else {
                    LogCollector.e(TAG, "Credential-based login failed")
                }
            } else {
                LogCollector.e(TAG, "No credentials available for authentication")
            }

            LogCollector.d(TAG, "Returning null (no valid token)")
            return null
        }

        /**
         * Force re-authentication to refresh the token.
         */
        fun refreshAuthToken(context: Context): String? {
            LogCollector.d(TAG, "Force refreshing auth token...")

            val username = SecureStorageHelper.getUsername(context)
            val password = SecureStorageHelper.getPassword(context)

            if (!username.isNullOrEmpty() && !password.isNullOrEmpty()) {
                val baseUrl = PreferenceHelper.getServerUrl(context)
                LogCollector.d(TAG, "Using credential-based authentication for refresh")

                if (loginWithCredentials(context, username, password, baseUrl)) {
                    LogCollector.d(TAG, "Credential-based refresh successful")
                    return SecureStorageHelper.getAccessToken(context)
                } else {
                    LogCollector.e(TAG, "Credential-based refresh failed")
                }
            } else {
                LogCollector.e(TAG, "No credentials available for refresh")
            }

            LogCollector.d(TAG, "Returning null (failed to refresh token)")
            return null
        }

        fun isTokenValid(token: String): Boolean {
            // Simplified JWT validation - check if token has 3 parts and is not expired
            val parts = token.split(".")
            if (parts.size != 3) {
                return false
            }
            
            try {
                // Decode payload (second part of JWT). JWTs use unpadded base64url — without
                // NO_PADDING, tokens whose payload length isn't a multiple of 4 fail to decode
                // and would be (incorrectly) treated as invalid.
                val payload = String(android.util.Base64.decode(parts[1], android.util.Base64.URL_SAFE or android.util.Base64.NO_PADDING))
                val json = JSONObject(payload)
                
                val exp = json.optLong("exp", 0)
                if (exp == 0L) {
                    return false
                }
                
                val now = System.currentTimeMillis() / 1000
                return exp > now
            } catch (e: Exception) {
                LogCollector.e(TAG, "Error validating token", e)
                return false
            }
        }
        
        /**
         * Discover the home server URL from the relay.
         * Calls GET /api/peers/home-server with the current JWT token.
         * On success, saves the home_local_url to preferences.
         * Fails silently (logs warning) — must not block login.
         */
        fun discoverHomeServer(context: Context, relayUrl: String) {
            try {
                val token = SecureStorageHelper.getAccessToken(context)
                if (token.isNullOrEmpty()) {
                    LogCollector.w(TAG, "discoverHomeServer: no token available")
                    return
                }

                val client = createHttpClient(relayUrl)
                val url = "${relayUrl.trimEnd('/')}/api/peers/home-server"

                val request = Request.Builder()
                    .url(url)
                    .get()
                    .addHeader("Authorization", "Bearer $token")
                    .build()

                val response = client.newCall(request).execute()

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
            } catch (e: Exception) {
                LogCollector.w(TAG, "discoverHomeServer failed: ${e.message}")
            }
        }

        fun pingServer(baseUrl: String): Boolean {
            return try {
                val client = createHttpClient(baseUrl).newBuilder()
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
            // In a real implementation, this would return a unique device identifier
            // For now, returning a placeholder
            return android.provider.Settings.Secure.getString(
                context.contentResolver,
                android.provider.Settings.Secure.ANDROID_ID
            ) ?: "unknown"
        }

        /**
         * Executes an HTTP request with automatic token refresh on 401 responses.
         * This eliminates duplicated token refresh logic across the codebase.
         *
         * @param context Application context
         * @param client OkHttpClient to use for the request
         * @param request The request to execute
         * @return Response from the server (either original or retry after token refresh)
         */
        fun executeWithTokenRefresh(
            context: Context,
            client: OkHttpClient,
            request: Request
        ): Response {
            var response = client.newCall(request).execute()

            // Check if the response is a 401, which means the token has expired
            if (response.code == 401) {
                LogCollector.e(TAG, "Received 401, attempting to refresh token...")

                // Try to refresh the auth token
                val newToken = refreshAuthToken(context)

                if (!newToken.isNullOrEmpty()) {
                    LogCollector.d(TAG, "Successfully refreshed token, retrying request...")

                    // Create a new request with the fresh token
                    val retryRequest = request.newBuilder()
                        .header("Authorization", "Bearer $newToken")
                        .build()

                    // Execute the retry request
                    response = client.newCall(retryRequest).execute()
                    LogCollector.d(TAG, "Retry request completed. Code: ${response.code}")
                } else {
                    LogCollector.e(TAG, "Failed to refresh token (session expired), redirecting to login")
                    redirectToLogin(context)
                }
            }

            return response
        }

        fun redirectToLogin(context: Context) {
            try {
                SecureStorageHelper.clearCredentials(context)
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