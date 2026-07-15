package org.openreminisce.app.util

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.Build
import okhttp3.OkHttpClient
import javax.net.ssl.*
import java.security.cert.X509Certificate

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
     * Determines if the URL is a private/local IP address or localhost.
     */
    fun isPrivateOrLocalAddress(url: String): Boolean {
        val host = try {
            java.net.URL(url).host.lowercase()
        } catch (e: Exception) {
            return false
        }

        // Check for localhost
        if (host == "localhost" || host == "127.0.0.1" || host == "::1") {
            return true
        }

        // Check private IPv4
        if (host.matches(Regex("^\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}$"))) {
            val parts = host.split(".").mapNotNull { it.toIntOrNull() }
            if (parts.size == 4) {
                val first = parts[0]
                val second = parts[1]
                return first == 10 ||
                       (first == 172 && second in 16..31) ||
                       (first == 192 && second == 168) ||
                       (first == 169 && second == 254)
            }
        }

        // Check private IPv6
        if (host.contains(":")) {
            val cleanHost = host.lowercase()
            return cleanHost.startsWith("fe80:") ||
                   cleanHost.startsWith("fc00:") ||
                   cleanHost.startsWith("fd00:")
        }

        return false
    }

    /**
     * Configures insecure SSL trust managers and hostnames for private/local IP networks.
     */
    fun configureInsecureSsl(builder: OkHttpClient.Builder, url: String) {
        if (isPrivateOrLocalAddress(url)) {
            val sslContext = SSLContext.getInstance("TLS")
            val trustManager = TrustAllCerts()
            sslContext.init(null, arrayOf(trustManager), java.security.SecureRandom())
            val tlsSocketFactory = TLSSocketFactory(sslContext.socketFactory)
            builder
                .sslSocketFactory(tlsSocketFactory, trustManager)
                .hostnameVerifier { _, _ -> true }
        }
    }

    // Custom TrustManager that accepts all certificates (for self-signed certificates on local network only)
    class TrustAllCerts : X509TrustManager {
        override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) {}
        override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) {}
        override fun getAcceptedIssuers(): Array<X509Certificate> = arrayOf()
    }

    // Custom SSLSocketFactory that enables TLS 1.2 and 1.3
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
                socket.enabledProtocols = arrayOf("TLSv1.2", "TLSv1.3")
            }
            return socket
        }
    }
}