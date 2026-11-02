package org.openreminisce.app.util

import android.content.Context
import android.util.Log
import com.bumptech.glide.Glide
import com.bumptech.glide.Registry
import com.bumptech.glide.annotation.Excludes
import com.bumptech.glide.annotation.GlideModule
import com.bumptech.glide.integration.okhttp3.OkHttpLibraryGlideModule
import com.bumptech.glide.integration.okhttp3.OkHttpUrlLoader
import com.bumptech.glide.load.model.GlideUrl
import com.bumptech.glide.module.AppGlideModule
import okhttp3.Call
import okhttp3.Interceptor
import okhttp3.OkHttpClient
import okhttp3.Response
import java.io.InputStream
import java.util.concurrent.TimeUnit

// This is a utility class to handle authentication for HTTP requests
// Caches the token to avoid repeated reads from EncryptedSharedPreferences
class ThumbnailAuthInterceptor(private val context: Context) : Interceptor {
    companion object {
        private const val TAG = "ThumbnailAuthInterceptor"
        @Volatile
        private var cachedToken: String? = null
        @Volatile
        private var tokenExpiryTime: Long = 0

        fun clearCachedToken() {
            cachedToken = null
            tokenExpiryTime = 0
        }
    }

    private fun getCachedToken(): String? {
        val now = System.currentTimeMillis()

        // If token is cached and not near expiry (5 min buffer), use it
        if (cachedToken != null && tokenExpiryTime > now + 300_000) {
            return cachedToken
        }

        // Otherwise fetch from storage (this is expensive, do it rarely)
        val token = AuthHelper.getValidToken(context)
        if (token != null) {
            cachedToken = token
            // Parse expiry from JWT
            try {
                val parts = token.split(".")
                if (parts.size == 3) {
                    val payload = String(android.util.Base64.decode(parts[1], android.util.Base64.URL_SAFE or android.util.Base64.NO_PADDING))
                    val json = org.json.JSONObject(payload)
                    tokenExpiryTime = json.optLong("exp", 0) * 1000
                }
            } catch (e: Exception) {
                LogCollector.e(TAG, "Error parsing token expiry", e)
                // Default to 1 hour from now if parsing fails
                tokenExpiryTime = now + 3600_000
            }
        }

        return token
    }

    override fun intercept(chain: Interceptor.Chain): Response {
        val originalRequest = chain.request()

        // If authorization header is already present (e.g. injected via GlideUrl LazyHeaders), proceed
        if (!originalRequest.header("Authorization").isNullOrEmpty()) {
            return chain.proceed(originalRequest)
        }

        // Only add the header for thumbnail or image requests
        val path = originalRequest.url.encodedPath
        val newRequest = if (path.contains("/api/thumbnail/") || path.contains("/api/image/")) {
            val token = getCachedToken()
            if (!token.isNullOrEmpty()) {
                originalRequest.newBuilder()
                    .addHeader("Authorization", "Bearer $token")
                    .build()
            } else {
                originalRequest
            }
        } else {
            originalRequest
        }

        val response = chain.proceed(newRequest)

        // If we get 401, clear the cached token so it gets refreshed
        if (response.code == 401) {
            LogCollector.w(TAG, "Got 401 on ${originalRequest.url}, clearing cached token")
            clearCachedToken()
        }

        return response
    }
}

object AuthenticatedHttpClient {
    private const val TAG = "AuthenticatedHttpClient"
    @Volatile
    private var client: OkHttpClient? = null
    @Volatile
    private var cachedUrl: String? = null
    private val lock = Any()

    fun getClient(context: Context): OkHttpClient {
        val baseUrl = PreferenceHelper.getServerUrl(context)

        // Clear cache if URL changed
        if (client != null && cachedUrl != baseUrl) {
            LogCollector.d(TAG, "URL changed from $cachedUrl to $baseUrl - clearing cached client")
            synchronized(lock) {
                client = null
            }
        }

        client?.let { return it }

        return synchronized(lock) {
            client?.let { return it }

            val useInsecure = NetworkHelper.isPrivateOrLocalAddress(baseUrl)
            cachedUrl = baseUrl

            LogCollector.d(TAG, "getClient called - Creating client for URL: $baseUrl (useInsecure: $useInsecure)")

            val builder = OkHttpClient.Builder()
                .addInterceptor(ThumbnailAuthInterceptor(context.applicationContext))
                .connectTimeout(30, TimeUnit.SECONDS)
                .readTimeout(30, TimeUnit.SECONDS)
                .addNetworkInterceptor { chain ->
                    val request = chain.request()
                    LogCollector.d(TAG, "Network request to: ${request.url}")
                    try {
                        val response = chain.proceed(request)
                        LogCollector.d(TAG, "Network response: ${response.code}")
                        response
                    } catch (e: Exception) {
                        LogCollector.e(TAG, "Network error in interceptor", e)
                        throw e
                    }
                }

            NetworkHelper.configureInsecureSsl(builder, baseUrl)

            val newClient = builder.build()
            client = newClient
            LogCollector.d(TAG, "Client created successfully")
            newClient
        }
    }

    /**
     * Creates a new OkHttpClient with custom timeout settings.
     * Use this for long-running operations like file uploads.
     */
    fun getClientWithTimeouts(
        context: Context,
        connectTimeoutSeconds: Long = 30,
        readTimeoutSeconds: Long = 300
    ): OkHttpClient {
        val baseUrl = PreferenceHelper.getServerUrl(context)
        val useInsecure = NetworkHelper.isPrivateOrLocalAddress(baseUrl)

        LogCollector.d(TAG, "Creating client with custom timeouts for URL: $baseUrl (useInsecure: $useInsecure)")

        val builder = OkHttpClient.Builder()
            .addInterceptor(ThumbnailAuthInterceptor(context.applicationContext))
            .connectTimeout(connectTimeoutSeconds, TimeUnit.SECONDS)
            .readTimeout(readTimeoutSeconds, TimeUnit.SECONDS)

        NetworkHelper.configureInsecureSsl(builder, baseUrl)

        return builder.build()
    }
}

/**
 * Custom Glide module that registers the authenticated OkHttpClient.
 * Dynamically resolves the active OkHttpClient from AuthenticatedHttpClient on every call.
 */
@GlideModule
@Excludes(OkHttpLibraryGlideModule::class)
class MyAppGlideModule : AppGlideModule() {
    override fun registerComponents(context: Context, glide: Glide, registry: Registry) {
        val appContext = context.applicationContext
        val callFactory = Call.Factory { request ->
            AuthenticatedHttpClient.getClient(appContext).newCall(request)
        }
        registry.replace(
            GlideUrl::class.java,
            InputStream::class.java,
            OkHttpUrlLoader.Factory(callFactory)
        )
        LogCollector.d("MyAppGlideModule", "Registered dynamic authenticated OkHttpClient with Glide")
    }

    override fun isManifestParsingEnabled(): Boolean {
        // Disable manifest parsing to avoid adding default integration
        return false
    }
}
