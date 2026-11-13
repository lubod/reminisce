package org.openreminisce.app.util

import android.content.Context
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

// Caches the active JWT access token for Glide/OkHttp media requests and refreshes once on 401.
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

        private fun updateCachedToken(token: String?) {
            if (token.isNullOrEmpty()) {
                clearCachedToken()
                return
            }
            cachedToken = token
            val parsedExpiry = AuthHelper.decodeJwtExpiryMs(token)
            tokenExpiryTime = if (parsedExpiry > 0L) parsedExpiry else System.currentTimeMillis() + 3600_000L
        }
    }

    private fun getCachedToken(): String? {
        val now = System.currentTimeMillis()
        val existing = cachedToken
        if (!existing.isNullOrEmpty() && tokenExpiryTime > now + 300_000) {
            return existing
        }

        val token = AuthHelper.getValidToken(context)
        updateCachedToken(token)
        return token
    }

    override fun intercept(chain: Interceptor.Chain): Response {
        val originalRequest = chain.request()
        val path = originalRequest.url.encodedPath
        val isAuthEndpoint = path.contains("/api/auth/")

        // Callers that already attach Authorization (e.g. via AuthHelper.executeWithTokenRefresh)
        // manage their own retry loop; we only invalidate the cached thumbnail token on 401.
        if (!originalRequest.header("Authorization").isNullOrEmpty()) {
            val response = chain.proceed(originalRequest)
            if (response.code == 401 && !isAuthEndpoint) {
                clearCachedToken()
            }
            return response
        }

        val shouldInjectAuth = !isAuthEndpoint && (
            path.contains("/api/thumbnail/") ||
                path.contains("/api/image/") ||
                path.contains("/api/video/")
            )

        val injectedToken = if (shouldInjectAuth) getCachedToken() else null
        val newRequest = if (!injectedToken.isNullOrEmpty()) {
            originalRequest.newBuilder()
                .addHeader("Authorization", "Bearer $injectedToken")
                .build()
        } else {
            originalRequest
        }

        val response = chain.proceed(newRequest)

        if (response.code == 401 && shouldInjectAuth && !isAuthEndpoint) {
            LogCollector.w(TAG, "Got 401 on ${originalRequest.url}, refreshing access token")
            clearCachedToken()
            val refreshedToken = AuthHelper.refreshAuthToken(context, failedAccessToken = injectedToken)
            if (!refreshedToken.isNullOrEmpty()) {
                updateCachedToken(refreshedToken)
                response.close()
                val retryRequest = originalRequest.newBuilder()
                    .header("Authorization", "Bearer $refreshedToken")
                    .build()
                return chain.proceed(retryRequest)
            } else if (SecureStorageHelper.getRefreshToken(context).isNullOrEmpty()) {
                AuthHelper.redirectToLogin(context)
            }
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

    fun invalidateClient() {
        synchronized(lock) {
            client = null
            cachedUrl = null
        }
    }

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

            val isPrivateOrLocal = NetworkHelper.isPrivateOrLocalAddress(baseUrl)
            cachedUrl = baseUrl

            LogCollector.d(TAG, "getClient called - Creating client for URL: $baseUrl (isPrivateOrLocal: $isPrivateOrLocal)")

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

            NetworkHelper.configureSsl(builder, baseUrl, context)

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
        val isPrivateOrLocal = NetworkHelper.isPrivateOrLocalAddress(baseUrl)

        LogCollector.d(TAG, "Creating client with custom timeouts for URL: $baseUrl (isPrivateOrLocal: $isPrivateOrLocal)")

        val builder = OkHttpClient.Builder()
            .addInterceptor(ThumbnailAuthInterceptor(context.applicationContext))
            .connectTimeout(connectTimeoutSeconds, TimeUnit.SECONDS)
            .readTimeout(readTimeoutSeconds, TimeUnit.SECONDS)

        NetworkHelper.configureSsl(builder, baseUrl, context)

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
