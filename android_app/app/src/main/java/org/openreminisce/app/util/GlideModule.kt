package org.openreminisce.app.util

import okhttp3.Interceptor
import okhttp3.OkHttpClient
import okhttp3.Response
import android.content.Context
import android.util.Log
import java.util.concurrent.TimeUnit
import javax.net.ssl.*
import java.security.cert.X509Certificate
import com.bumptech.glide.Glide
import com.bumptech.glide.GlideBuilder
import com.bumptech.glide.Registry
import com.bumptech.glide.annotation.GlideModule
import com.bumptech.glide.integration.okhttp3.OkHttpUrlLoader
import com.bumptech.glide.load.model.GlideUrl
import com.bumptech.glide.module.AppGlideModule
import java.io.InputStream

// Custom TrustManager that accepts all certificates (for self-signed certificates)
class TrustAllCerts : X509TrustManager {
    override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) {}
    override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) {}
    override fun getAcceptedIssuers(): Array<X509Certificate> = arrayOf()
}

// Custom SSLSocketFactory that enables all TLS protocols
class TLSSocketFactory(private val delegate: SSLSocketFactory) : SSLSocketFactory() {
    override fun getDefaultCipherSuites(): Array<String> = delegate.defaultCipherSuites
    override fun getSupportedCipherSuites(): Array<String> = delegate.supportedCipherSuites

    override fun createSocket(s: java.net.Socket?, host: String?, port: Int, autoClose: Boolean): java.net.Socket {
        val socket = delegate.createSocket(s, host, port, autoClose)
        return enableTLSOnSocket(socket)
    }

    override fun createSocket(host: String?, port: Int): java.net.Socket {
        val socket = delegate.createSocket(host, port)
        return enableTLSOnSocket(socket)
    }

    override fun createSocket(host: String?, port: Int, localHost: java.net.InetAddress?, localPort: Int): java.net.Socket {
        val socket = delegate.createSocket(host, port, localHost, localPort)
        return enableTLSOnSocket(socket)
    }

    override fun createSocket(host: java.net.InetAddress?, port: Int): java.net.Socket {
        val socket = delegate.createSocket(host, port)
        return enableTLSOnSocket(socket)
    }

    override fun createSocket(address: java.net.InetAddress?, port: Int, localAddress: java.net.InetAddress?, localPort: Int): java.net.Socket {
        val socket = delegate.createSocket(address, port, localAddress, localPort)
        return enableTLSOnSocket(socket)
    }

    private fun enableTLSOnSocket(socket: java.net.Socket): java.net.Socket {
        if (socket is SSLSocket) {
            // Enable only modern TLS protocols (TLS 1.2 and 1.3)
            // Legacy TLS 1.0 and 1.1 are vulnerable and should not be used
            socket.enabledProtocols = arrayOf("TLSv1.2", "TLSv1.3")
        }
        return socket
    }
}

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
                    val payload = String(android.util.Base64.decode(parts[1], android.util.Base64.URL_SAFE))
                    val json = org.json.JSONObject(payload)
                    tokenExpiryTime = json.optLong("exp", 0) * 1000
                }
            } catch (e: Exception) {
                Log.e(TAG, "Error parsing token expiry", e)
                // Default to 1 hour from now if parsing fails
                tokenExpiryTime = now + 3600_000
            }
        }

        return token
    }

    override fun intercept(chain: Interceptor.Chain): Response {
        val originalRequest = chain.request()

        // Only add the header for thumbnail requests
        val newRequest = if (originalRequest.url.encodedPath.contains("/api/thumbnail/")) {
            val token = getCachedToken()
            if (token != null) {
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
            Log.w(TAG, "Got 401, clearing cached token")
            clearCachedToken()
        }

        return response
    }
}

object AuthenticatedHttpClient {
    private const val TAG = "AuthenticatedHttpClient"
    private var client: OkHttpClient? = null
    private var cachedUrl: String? = null

    /**
     * Determines if the URL is an IP address or localhost.
     * Returns true for IP addresses and localhost, false for domain names.
     */
    private fun isIpAddressOrLocalhost(url: String): Boolean {
        Log.d(TAG, "Checking URL type for: $url")
        val host = try {
            java.net.URL(url).host.lowercase()
        } catch (e: Exception) {
            Log.w(TAG, "Failed to parse URL: $url", e)
            return false
        }

        Log.d(TAG, "Extracted host: $host")

        // Check for localhost
        if (host == "localhost" || host == "127.0.0.1" || host == "::1") {
            Log.d(TAG, "Detected as localhost")
            return true
        }

        // Check for IPv4 address (basic pattern)
        val ipv4Pattern = Regex("^\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}$")
        if (ipv4Pattern.matches(host)) {
            Log.d(TAG, "Detected as IPv4 address")
            return true
        }

        // Check for IPv6 address (contains colons)
        if (host.contains(":") && !host.contains(".")) {
            Log.d(TAG, "Detected as IPv6 address")
            return true
        }

        // Check for private IP ranges
        if (host.startsWith("192.168.") || host.startsWith("10.") ||
            host.matches(Regex("^172\\.(1[6-9]|2[0-9]|3[0-1])\\..*"))) {
            Log.d(TAG, "Detected as private IP range")
            return true
        }

        Log.d(TAG, "Detected as domain name (will use secure client)")
        return false
    }

    fun getClient(context: Context): OkHttpClient {
        val baseUrl = PreferenceHelper.getServerUrl(context)

        // Clear cache if URL changed
        if (client != null && cachedUrl != baseUrl) {
            Log.d(TAG, "URL changed from $cachedUrl to $baseUrl - clearing cached client")
            client = null
        }

        if (client == null) {
            val useInsecure = isIpAddressOrLocalhost(baseUrl)
            cachedUrl = baseUrl

            Log.d(TAG, "getClient called - Creating client for URL: $baseUrl (useInsecure: $useInsecure)")

            val builder = OkHttpClient.Builder()
                .addInterceptor(ThumbnailAuthInterceptor(context.applicationContext))
                .connectTimeout(30, TimeUnit.SECONDS)
                .readTimeout(30, TimeUnit.SECONDS)
                .addNetworkInterceptor { chain ->
                    val request = chain.request()
                    Log.d(TAG, "Network request to: ${request.url}")
                    try {
                        val response = chain.proceed(request)
                        Log.d(TAG, "Network response: ${response.code}")
                        response
                    } catch (e: Exception) {
                        Log.e(TAG, "Network error in interceptor", e)
                        throw e
                    }
                }

            if (useInsecure) {
                Log.d(TAG, "Configuring insecure SSL for IP/localhost")
                // Create SSL context that trusts all certificates for IP/localhost
                val sslContext = SSLContext.getInstance("TLS")
                sslContext.init(null, arrayOf(TrustAllCerts()), java.security.SecureRandom())

                // Wrap with TLSSocketFactory to enable all TLS protocols
                val tlsSocketFactory = TLSSocketFactory(sslContext.socketFactory)

                builder
                    .sslSocketFactory(tlsSocketFactory, TrustAllCerts())
                    .hostnameVerifier { _, _ -> true } // Accept all hostnames
            } else {
                Log.d(TAG, "Using default secure SSL for domain name")
            }

            client = builder.build()
            Log.d(TAG, "Client created successfully")
        }
        return client!!
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
        val useInsecure = isIpAddressOrLocalhost(baseUrl)

        Log.d(TAG, "Creating client with custom timeouts for URL: $baseUrl (useInsecure: $useInsecure)")

        val builder = OkHttpClient.Builder()
            .addInterceptor(ThumbnailAuthInterceptor(context.applicationContext))
            .connectTimeout(connectTimeoutSeconds, TimeUnit.SECONDS)
            .readTimeout(readTimeoutSeconds, TimeUnit.SECONDS)

        if (useInsecure) {
            // Create SSL context that trusts all certificates for IP/localhost
            val sslContext = SSLContext.getInstance("TLS")
            sslContext.init(null, arrayOf(TrustAllCerts()), java.security.SecureRandom())

            // Wrap with TLSSocketFactory to enable all TLS protocols
            val tlsSocketFactory = TLSSocketFactory(sslContext.socketFactory)

            builder
                .sslSocketFactory(tlsSocketFactory, TrustAllCerts())
                .hostnameVerifier { _, _ -> true } // Accept all hostnames
        }

        return builder.build()
    }
}

/**
 * Custom Glide module that registers the authenticated OkHttpClient.
 * This ensures all Glide image requests include the auth token.
 */
@GlideModule
class MyAppGlideModule : AppGlideModule() {
    override fun registerComponents(context: Context, glide: Glide, registry: Registry) {
        val client = AuthenticatedHttpClient.getClient(context)
        registry.replace(
            GlideUrl::class.java,
            InputStream::class.java,
            OkHttpUrlLoader.Factory(client)
        )
        Log.d("MyAppGlideModule", "Registered authenticated OkHttpClient with Glide")
    }

    override fun isManifestParsingEnabled(): Boolean {
        // Disable manifest parsing to avoid adding default integration
        return false
    }
}