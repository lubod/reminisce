package org.openreminisce.app.util

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.net.wifi.WifiManager
import android.util.Log
import java.net.InetSocketAddress
import java.net.Socket

object NetworkScanner {
    private const val TAG = "NetworkScanner"
    private const val SCAN_PORT = 18443
    private const val CONNECTION_TIMEOUT_MS = 500 // Quick timeout for scanning

    sealed class ScanResult {
        data class Found(val ipAddress: String, val url: String) : ScanResult()
        object NotFound : ScanResult()
        data class Error(val message: String) : ScanResult()
    }

    /**
     * Scan the local network for a server on port 18443
     * Returns the first server found
     */
    fun scanForServer(context: Context, progressCallback: ((String) -> Unit)? = null): ScanResult {
        try {
            // Get the local IP address and subnet
            val localIp = getLocalIpAddress(context)
            if (localIp == null) {
                Log.e(TAG, "Could not get local IP address")
                return ScanResult.Error("Not connected to a local network. Please ensure you're connected to WiFi or Ethernet.")
            }

            Log.d(TAG, "Local IP: $localIp")
            progressCallback?.invoke("Scanning network...")

            // Extract the network prefix (e.g., 192.168.1)
            val ipParts = localIp.split(".")
            if (ipParts.size != 4) {
                return ScanResult.Error("Invalid IP address format")
            }

            val networkPrefix = "${ipParts[0]}.${ipParts[1]}.${ipParts[2]}"
            Log.d(TAG, "Scanning network: $networkPrefix.0/24")

            // Scan the network (excluding .0 and .255)
            for (i in 1..254) {
                val ipToTest = "$networkPrefix.$i"

                // Update progress occasionally
                if (i % 10 == 0) {
                    progressCallback?.invoke("Scanning $ipToTest...")
                }

                // Try to connect to the server
                if (isServerRunning(ipToTest, SCAN_PORT)) {
                    Log.d(TAG, "Found server at $ipToTest:$SCAN_PORT")
                    val url = "https://$ipToTest:$SCAN_PORT"

                    // Verify it's actually our backup server by testing the connection
                    progressCallback?.invoke("Found server at $ipToTest, testing...")
                    val testResult = ServerValidator.pingServer(context, url)

                    if (testResult is ServerValidator.PingResult.Success) {
                        Log.d(TAG, "Server verified at $url")
                        return ScanResult.Found(ipToTest, url)
                    } else {
                        Log.d(TAG, "Server at $ipToTest:$SCAN_PORT failed validation")
                        // Continue scanning for other servers
                    }
                }
            }

            Log.d(TAG, "No server found on network")
            return ScanResult.NotFound

        } catch (e: Exception) {
            Log.e(TAG, "Error scanning network", e)
            return ScanResult.Error(e.message ?: "Unknown error")
        }
    }

    /**
     * Check if a server is running on the specified IP and port
     */
    private fun isServerRunning(ipAddress: String, port: Int): Boolean {
        return try {
            Socket().use { socket ->
                socket.connect(InetSocketAddress(ipAddress, port), CONNECTION_TIMEOUT_MS)
                true
            }
        } catch (e: Exception) {
            false
        }
    }

    /**
     * Get the local IP address of the device
     */
    private fun getLocalIpAddress(context: Context): String? {
        try {
            val connectivityManager = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
            val network = connectivityManager.activeNetwork
            val capabilities = connectivityManager.getNetworkCapabilities(network)

            if (network == null || capabilities == null) {
                Log.e(TAG, "No active network connection")
                return null
            }

            // Log network transport types for debugging
            Log.d(TAG, "Network capabilities - WiFi: ${capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)}, " +
                    "Ethernet: ${capabilities.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)}, " +
                    "Cellular: ${capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)}")

            // Try to get IP from network interfaces first (more reliable)
            val ipFromInterfaces = getIpFromNetworkInterfaces()
            if (ipFromInterfaces != null) {
                Log.d(TAG, "Got IP from network interfaces: $ipFromInterfaces")
                return ipFromInterfaces
            }

            // If network interfaces failed, try WiFi-specific method
            if (capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) {
                Log.d(TAG, "Connected via WiFi, trying WiFi-specific method...")
                val wifiIp = getWifiIpAddress(context)
                if (wifiIp != null) {
                    return wifiIp
                }
            }

            Log.e(TAG, "Could not determine local IP address - not on a local network or IP not yet assigned")
            return null

        } catch (e: Exception) {
            Log.e(TAG, "Error getting local IP address", e)
            return null
        }
    }

    /**
     * Get IP address from WiFi
     */
    private fun getWifiIpAddress(context: Context): String? {
        try {
            val connectivityManager = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
            val network = connectivityManager.activeNetwork
            val linkProperties = connectivityManager.getLinkProperties(network)

            if (linkProperties != null) {
                for (linkAddress in linkProperties.linkAddresses) {
                    val address = linkAddress.address
                    if (address is java.net.Inet4Address) {
                        val ipAddress = address.hostAddress
                        if (ipAddress != null) {
                            Log.d(TAG, "WiFi IP address from ConnectivityManager: $ipAddress")
                            return ipAddress
                        }
                    }
                }
            }
            // Fallback for older systems if needed
            return getIpFromNetworkInterfaces()
        } catch (e: Exception) {
            Log.e(TAG, "Error getting WiFi IP address", e)
            return null
        }
    }

    /**
     * Get IP address from network interfaces (works for WiFi, Ethernet, etc.)
     */
    private fun getIpFromNetworkInterfaces(): String? {
        try {
            val interfaces = java.net.NetworkInterface.getNetworkInterfaces()
            while (interfaces.hasMoreElements()) {
                val networkInterface = interfaces.nextElement()

                // Skip loopback and down interfaces
                if (networkInterface.isLoopback || !networkInterface.isUp) {
                    continue
                }

                val addresses = networkInterface.inetAddresses
                while (addresses.hasMoreElements()) {
                    val address = addresses.nextElement()

                    // We want IPv4 addresses only
                    if (!address.isLoopbackAddress && address is java.net.Inet4Address) {
                        val ipAddress = address.hostAddress ?: continue

                        // Check if it's a local network IP (192.168.x.x, 10.x.x.x, or 172.16-31.x.x)
                        if (ipAddress.startsWith("192.168.") ||
                            ipAddress.startsWith("10.") ||
                            ipAddress.matches(Regex("172\\.(1[6-9]|2[0-9]|3[0-1])\\..*"))) {
                            Log.d(TAG, "Found local IP from network interface: $ipAddress")
                            return ipAddress
                        }
                    }
                }
            }

            Log.e(TAG, "No local network IP address found")
            return null
        } catch (e: Exception) {
            Log.e(TAG, "Error getting IP from network interfaces", e)
            return null
        }
    }
}