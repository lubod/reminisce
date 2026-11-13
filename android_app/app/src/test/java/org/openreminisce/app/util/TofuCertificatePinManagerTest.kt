package org.openreminisce.app.util

import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Test
import java.math.BigInteger
import java.security.Principal
import java.security.PublicKey
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import java.util.Date
import javax.net.ssl.X509TrustManager

class TofuCertificatePinManagerTest {

    @After
    fun cleanupMismatchState() {
        TofuPinningSupport.consumeLastMismatch()
    }

    @Test
    fun normalizesSha256Formats() {
        val hex = "ab".repeat(32)
        val colonUpper = hex.uppercase().chunked(2).joinToString(":")
        assertEquals(hex, TofuPinningSupport.normalizeSha256(hex))
        assertEquals(hex, TofuPinningSupport.normalizeSha256("sha256/$colonUpper"))
        assertEquals(hex, TofuPinningSupport.normalizeSha256("  $colonUpper  "))

        val base64 = java.util.Base64.getEncoder().encodeToString(ByteArray(32) { 0xAB.toByte() })
        assertEquals(hex, TofuPinningSupport.normalizeSha256("SHA256/$base64"))
        assertNull(TofuPinningSupport.normalizeSha256("not-a-pin"))
    }

    @Test
    fun extractsHostPortKeys() {
        assertEquals("192.168.1.55:28443", TofuPinningSupport.hostPortKey("https://192.168.1.55:28443/api/ping"))
        assertEquals("my-server.ts.net:443", TofuPinningSupport.hostPortKey("https://MY-SERVER.ts.net"))
        assertEquals("fd7a:115c:a1e0::1:28443", TofuPinningSupport.hostPortKey("https://[fd7a:115c:a1e0::1]:28443"))
    }

    @Test
    fun privateHostPinsFirstSelfSignedCertAndDetectsChanges() {
        val store = InMemoryCertificatePinStore()
        val rejectingSystemTm = fakeTrustManager(shouldSucceed = false)
        val certA = TestX509Certificate("cert-alpha".encodeToByteArray())
        val certB = TestX509Certificate("cert-beta".encodeToByteArray())
        val hostPort = "192.168.1.100:28443"

        // First connection trusts-on-first-use and stores SHA-256
        TofuPinningSupport.verifyServerChain(arrayOf(certA), "RSA", hostPort, true, store, rejectingSystemTm)
        assertEquals(TofuPinningSupport.certificateSha256Hex(certA), store.getPin(hostPort))

        // Repeat connection with identical certificate succeeds
        TofuPinningSupport.verifyServerChain(arrayOf(certA), "RSA", hostPort, true, store, rejectingSystemTm)

        // Rotated / MITM certificate triggers CertificateMismatchException
        val mismatch = assertThrows(CertificateMismatchException::class.java) {
            TofuPinningSupport.verifyServerChain(arrayOf(certB), "RSA", hostPort, true, store, rejectingSystemTm)
        }
        assertEquals(hostPort, mismatch.hostPort)
        assertEquals(TofuPinningSupport.certificateSha256Hex(certA), mismatch.expectedSha256)
        assertEquals(TofuPinningSupport.certificateSha256Hex(certB), mismatch.actualSha256)
        assertNotNull(TofuPinningSupport.consumeLastMismatch())
    }

    @Test
    fun caTrustedTailscaleDomainDoesNotSaveTofuPinUnlessExplicitlyPinned() {
        val store = InMemoryCertificatePinStore()
        val acceptingSystemTm = fakeTrustManager(shouldSucceed = true)
        val certA = TestX509Certificate("letsencrypt-cert-1".encodeToByteArray())
        val certB = TestX509Certificate("letsencrypt-cert-2".encodeToByteArray())
        val hostPort = "home.tail1234.ts.net:443"

        // Standard CA chain succeeds cleanly -> no leaf pin stored, renewal passes smoothly
        TofuPinningSupport.verifyServerChain(arrayOf(certA), "RSA", hostPort, true, store, acceptingSystemTm)
        assertNull(store.getPin(hostPort))
        TofuPinningSupport.verifyServerChain(arrayOf(certB), "RSA", hostPort, true, store, acceptingSystemTm)
    }

    @Test
    fun publicHostStrictlyRejectsSelfSignedCert() {
        val store = InMemoryCertificatePinStore()
        val rejectingSystemTm = fakeTrustManager(shouldSucceed = false)
        val cert = TestX509Certificate("untrusted-public-cert".encodeToByteArray())

        assertThrows(CertificateException::class.java) {
            TofuPinningSupport.verifyServerChain(arrayOf(cert), "RSA", "example.com:443", false, store, rejectingSystemTm)
        }
        assertNull(store.getPin("example.com:443"))
    }

    @Test
    fun qrExplicitPinOverridesCaAndEnforcesMatch() {
        val store = InMemoryCertificatePinStore()
        val acceptingSystemTm = fakeTrustManager(shouldSucceed = true)
        val certA = TestX509Certificate("qr-pinned-cert".encodeToByteArray())
        val certB = TestX509Certificate("other-cert".encodeToByteArray())
        val hostPort = "100.100.50.25:28443"

        store.setPin(hostPort, TofuPinningSupport.certificateSha256Hex(certA))
        TofuPinningSupport.verifyServerChain(arrayOf(certA), "RSA", hostPort, true, store, acceptingSystemTm)

        assertThrows(CertificateMismatchException::class.java) {
            TofuPinningSupport.verifyServerChain(arrayOf(certB), "RSA", hostPort, true, store, acceptingSystemTm)
        }
    }

    private fun fakeTrustManager(shouldSucceed: Boolean): X509TrustManager = object : X509TrustManager {
        override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) {}
        override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) {
            if (!shouldSucceed) throw CertificateException("Untrusted anchor")
        }
        override fun getAcceptedIssuers(): Array<X509Certificate> = arrayOf()
    }

    private class TestX509Certificate(private val raw: ByteArray) : X509Certificate() {
        override fun getEncoded(): ByteArray = raw
        override fun verify(key: PublicKey?) {}
        override fun verify(key: PublicKey?, sigProvider: String?) {}
        override fun toString(): String = "TestX509Certificate(${raw.size})"
        override fun getPublicKey(): PublicKey = throw UnsupportedOperationException()
        override fun getBasicConstraints(): Int = -1
        override fun getKeyUsage(): BooleanArray? = null
        override fun getVersion(): Int = 3
        override fun getSerialNumber(): BigInteger = BigInteger.ONE
        override fun getIssuerDN(): Principal = Principal { "CN=test" }
        override fun getSubjectDN(): Principal = Principal { "CN=test" }
        override fun getNotBefore(): Date = Date(0)
        override fun getNotAfter(): Date = Date(Long.MAX_VALUE)
        override fun getTBSCertificate(): ByteArray = raw
        override fun getSignature(): ByteArray = byteArrayOf()
        override fun getSigAlgName(): String = "SHA256withRSA"
        override fun getSigAlgOID(): String = "1.2.840.113549.1.1.11"
        override fun getSigAlgParams(): ByteArray? = null
        override fun getIssuerUniqueID(): BooleanArray? = null
        override fun getSubjectUniqueID(): BooleanArray? = null
        override fun getNonCriticalExtensionOIDs(): MutableSet<String> = mutableSetOf()
        override fun getCriticalExtensionOIDs(): MutableSet<String> = mutableSetOf()
        override fun getExtensionValue(oid: String?): ByteArray? = null
        override fun hasUnsupportedCriticalExtension(): Boolean = false
        override fun checkValidity() {}
        override fun checkValidity(date: Date?) {}
    }
}
