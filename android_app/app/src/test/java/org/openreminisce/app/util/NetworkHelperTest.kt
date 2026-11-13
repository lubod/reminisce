package org.openreminisce.app.util

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class NetworkHelperTest {

    @Test
    fun testIsPrivateOrLocalAddress() {
        // Localhost
        assertTrue(NetworkHelper.isPrivateOrLocalAddress("http://localhost:8080"))
        assertTrue(NetworkHelper.isPrivateOrLocalAddress("https://127.0.0.1:28443"))
        assertTrue(NetworkHelper.isPrivateOrLocalAddress("http://[::1]:8080"))

        // Standard private IPv4
        assertTrue(NetworkHelper.isPrivateOrLocalAddress("https://192.168.1.197:28443"))
        assertTrue(NetworkHelper.isPrivateOrLocalAddress("http://192.168.0.1"))
        assertTrue(NetworkHelper.isPrivateOrLocalAddress("192.168.1.197:28443")) // without scheme
        assertTrue(NetworkHelper.isPrivateOrLocalAddress("http://10.0.0.1:8080"))
        assertTrue(NetworkHelper.isPrivateOrLocalAddress("https://172.16.0.1:443"))
        assertTrue(NetworkHelper.isPrivateOrLocalAddress("https://172.31.255.255:443"))
        assertTrue(NetworkHelper.isPrivateOrLocalAddress("http://169.254.1.1"))

        // Tailscale / NetBird / CGNAT range (100.64.0.0/10), IPv6 ULA, and MagicDNS
        assertTrue(NetworkHelper.isPrivateOrLocalAddress("https://100.64.0.1:28443"))
        assertTrue(NetworkHelper.isPrivateOrLocalAddress("https://100.100.50.25:28443"))
        assertTrue(NetworkHelper.isPrivateOrLocalAddress("https://100.127.255.254:28443"))
        assertTrue(NetworkHelper.isPrivateOrLocalAddress("https://my-server.tail1234.ts.net:28443"))
        assertTrue(NetworkHelper.isPrivateOrLocalAddress("https://[fd7a:115c:a1e0::1]:28443"))

        // isTailscaleAddress checks
        assertTrue(NetworkHelper.isTailscaleAddress("https://my-server.tail1234.ts.net:28443"))
        assertTrue(NetworkHelper.isTailscaleAddress("https://100.100.50.25:28443"))
        assertTrue(NetworkHelper.isTailscaleAddress("https://[fd7a:115c:a1e0::1]:28443"))
        assertFalse(NetworkHelper.isTailscaleAddress("https://192.168.1.197:28443"))
        assertFalse(NetworkHelper.isTailscaleAddress("https://100.128.0.1:28443"))
        assertFalse(NetworkHelper.isTailscaleAddress("https://example.com"))

        // Local domain suffixes
        assertTrue(NetworkHelper.isPrivateOrLocalAddress("https://reminisce.local:28443"))
        assertTrue(NetworkHelper.isPrivateOrLocalAddress("http://server.lan:8080"))
        assertTrue(NetworkHelper.isPrivateOrLocalAddress("http://nas.home:8080"))
        assertTrue(NetworkHelper.isPrivateOrLocalAddress("http://nas.internal:8080"))

        // Public / non-local addresses should return false
        assertFalse(NetworkHelper.isPrivateOrLocalAddress("https://example.com"))
        assertFalse(NetworkHelper.isPrivateOrLocalAddress("https://8.8.8.8:443"))
        assertFalse(NetworkHelper.isPrivateOrLocalAddress("https://1.1.1.1"))
        assertFalse(NetworkHelper.isPrivateOrLocalAddress("https://172.32.0.1")) // outside 172.16-31
        assertFalse(NetworkHelper.isPrivateOrLocalAddress("https://100.128.0.1")) // outside 100.64/10
        assertFalse(NetworkHelper.isPrivateOrLocalAddress(""))
    }
}
