package org.openreminisce.app.util

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.Build
import okhttp3.OkHttpClient
import java.net.Socket
import java.security.SecureRandom
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket
import javax.net.ssl.SSLSocketFactory

object NetworkHelper {
    /**
     * Check if device has active internet connection
     */
    fun isNetworkAvailable(context: Context): Boolean {
        val connectivityManager = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager

        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            val network = connectivityManager.activeNetwork ?: return false
            val capabilities = connectivityManager.getNetworkCapabilities(network) ?: return false

            capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) &&
                capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
        } else {
            @Suppress("DEPRECATION")
            val networkInfo = connectivityManager.activeNetworkInfo
            @Suppress("DEPRECATION")
            networkInfo?.isConnected == true
        }
    }

    /**
     * Check if device is connected to WiFi
     */
    fun isWifiConnected(context: Context): Boolean {
        val connectivityManager = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager

        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            val network = connectivityManager.activeNetwork ?: return false
            val capabilities = connectivityManager.getNetworkCapabilities(network) ?: return false
            capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)
        } else {
            @Suppress("DEPRECATION")
            val networkInfo = connectivityManager.activeNetworkInfo
            @Suppress("DEPRECATION")
            networkInfo?.type == ConnectivityManager.TYPE_WIFI && networkInfo.isConnected
        }
    }

    /**
     * Extracts a lowercase normalized host (without IPv6 brackets) from a URL or host[:port] string.
     */
    private fun extractHost(url: String): String? {
        if (url.isBlank()) return null
        val cleanUrl = if (!url.contains("://")) "http://$url" else url
        val rawHost = try {
            java.net.URL(cleanUrl).host?.lowercase() ?: return null
        } catch (e: Exception) {
            return null
        }
        return rawHost.removePrefix("[").removeSuffix("]")
    }

    /**
     * Determines whether the URL targets a Tailscale IP (100.64.0.0/10, fd7a:115c:a1e0::/48)
     * or MagicDNS hostname (*.ts.net).
     */
    fun isTailscaleAddress(url: String): Boolean {
        val host = extractHost(url) ?: return false
        if (host.endsWith(".ts.net")) return true
        if (host.startsWith("fd7a:115c:a1e0:")) return true
        if (host.matches(Regex("^\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}$"))) {
            val parts = host.split(".").mapNotNull { it.toIntOrNull() }
            if (parts.size == 4) {
                return parts[0] == 100 && parts[1] in 64..127
            }
        }
        return false
    }

    /**
     * Determines if the URL is a private/local IP address, local domain, Tailscale address, or localhost.
     */
    fun isPrivateOrLocalAddress(url: String): Boolean {
        val host = extractHost(url) ?: return false

        if (host == "localhost" || host == "127.0.0.1" || host == "::1") {
            return true
        }

        if (host.endsWith(".local") || host.endsWith(".lan") || host.endsWith(".home") ||
            host.endsWith(".internal") || host.endsWith(".ts.net")) {
            return true
        }

        if (host.matches(Regex("^\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}$"))) {
            val parts = host.split(".").mapNotNull { it.toIntOrNull() }
            if (parts.size == 4) {
                val first = parts[0]
                val second = parts[1]
                return first == 10 ||
                    (first == 172 && second in 16..31) ||
                    (first == 192 && second == 168) ||
                    (first == 169 && second == 254) ||
                    (first == 100 && second in 64..127)
            }
        }

        if (host.contains(":")) {
            return host.startsWith("fe80:") ||
                host.startsWith("fc00:") ||
                host.startsWith("fd00:") ||
                host.startsWith("fd7a:")
        }

        return false
    }

    /**
     * Configures TLS verification with TOFU SHA-256 certificate pinning for private/local/Tailscale
     * HTTPS URLs while keeping strict system CA validation for public hosts.
     */
    fun configureSsl(builder: OkHttpClient.Builder, url: String, context: Context) {
        configureSsl(builder, url, SecureStorageHelper.createPinStore(context))
    }

    fun configureSsl(builder: OkHttpClient.Builder, url: String, pinStore: CertificatePinStore) {
        if (!url.trim().lowercase().startsWith("https://")) return
        val hostPort = TofuPinningSupport.hostPortKey(url) ?: return
        val isPrivateOrLocal = isPrivateOrLocalAddress(url)

        val trustManager = TofuTrustManager(
            hostPort = hostPort,
            isPrivateOrLocal = isPrivateOrLocal,
            pinStore = pinStore,
        )
        val sslContext = SSLContext.getInstance("TLS")
        sslContext.init(null, arrayOf(trustManager), SecureRandom())
        val tlsSocketFactory = TLSSocketFactory(sslContext.socketFactory)

        builder
            .sslSocketFactory(tlsSocketFactory, trustManager)
            .hostnameVerifier(
                TofuHostnameVerifier(
                    hostPort = hostPort,
                    isPrivateOrLocal = isPrivateOrLocal,
                    pinStore = pinStore,
                )
            )
    }

    // Custom SSLSocketFactory that enables TLS 1.2 and 1.3
    class TLSSocketFactory(private val delegate: SSLSocketFactory) : SSLSocketFactory() {
        override fun getDefaultCipherSuites(): Array<String> = delegate.defaultCipherSuites
        override fun getSupportedCipherSuites(): Array<String> = delegate.supportedCipherSuites

        override fun createSocket(s: Socket?, host: String?, port: Int, autoClose: Boolean): Socket {
            return enableTLSOnSocket(delegate.createSocket(s, host, port, autoClose))
        }

        override fun createSocket(host: String?, port: Int): Socket {
            return enableTLSOnSocket(delegate.createSocket(host, port))
        }

        override fun createSocket(host: String?, port: Int, localHost: java.net.InetAddress?, localPort: Int): Socket {
            return enableTLSOnSocket(delegate.createSocket(host, port, localHost, localPort))
        }

        override fun createSocket(host: java.net.InetAddress?, port: Int): Socket {
            return enableTLSOnSocket(delegate.createSocket(host, port))
        }

        override fun createSocket(address: java.net.InetAddress?, port: Int, localAddress: java.net.InetAddress?, localPort: Int): Socket {
            return enableTLSOnSocket(delegate.createSocket(address, port, localAddress, localPort))
        }

        private fun enableTLSOnSocket(socket: Socket): Socket {
            if (socket is SSLSocket) {
                socket.enabledProtocols = arrayOf("TLSv1.2", "TLSv1.3")
            }
            return socket
        }
    }
}
