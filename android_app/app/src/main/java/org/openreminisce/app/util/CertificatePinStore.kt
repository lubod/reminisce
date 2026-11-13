package org.openreminisce.app.util

import android.app.Activity
import android.net.http.SslCertificate
import androidx.appcompat.app.AlertDialog
import okhttp3.internal.tls.OkHostnameVerifier
import okio.ByteString.Companion.decodeBase64
import java.io.ByteArrayInputStream
import java.net.URL
import java.security.KeyStore
import java.security.MessageDigest
import java.security.cert.CertificateException
import java.security.cert.CertificateFactory
import java.security.cert.X509Certificate
import javax.net.ssl.HostnameVerifier
import javax.net.ssl.HttpsURLConnection
import javax.net.ssl.SSLPeerUnverifiedException
import javax.net.ssl.SSLSession
import javax.net.ssl.TrustManagerFactory
import javax.net.ssl.X509TrustManager

interface CertificatePinStore {
    fun getPin(hostPort: String): String?
    fun setPin(hostPort: String, sha256Hex: String)
    fun clearPin(hostPort: String)
}

class InMemoryCertificatePinStore : CertificatePinStore {
    private val pins = mutableMapOf<String, String>()

    @Synchronized
    override fun getPin(hostPort: String): String? = pins[hostPort]

    @Synchronized
    override fun setPin(hostPort: String, sha256Hex: String) {
        TofuPinningSupport.normalizeSha256(sha256Hex)?.let { normalized ->
            pins[hostPort] = normalized
        }
    }

    @Synchronized
    override fun clearPin(hostPort: String) {
        pins.remove(hostPort)
    }
}

class CertificateMismatchException(
    val hostPort: String,
    val expectedSha256: String,
    val actualSha256: String,
) : CertificateException(
    "Certificate changed for $hostPort (expected ${TofuPinningSupport.formatFingerprint(expectedSha256)}, " +
        "got ${TofuPinningSupport.formatFingerprint(actualSha256)})"
)

object TofuPinningSupport {
    @Volatile
    private var lastMismatch: CertificateMismatchException? = null

    fun recordMismatch(mismatch: CertificateMismatchException) {
        lastMismatch = mismatch
    }

    fun consumeLastMismatch(): CertificateMismatchException? {
        val current = lastMismatch
        lastMismatch = null
        return current
    }

    fun findMismatch(throwable: Throwable?): CertificateMismatchException? {
        var cur = throwable
        while (cur != null) {
            if (cur is CertificateMismatchException) return cur
            cur = cur.cause
        }
        return lastMismatch
    }

    fun hostPortKey(url: String): String? {
        if (url.isBlank()) return null
        val clean = if (!url.contains("://")) "https://$url" else url.trim()
        return try {
            val parsed = URL(clean)
            val host = parsed.host?.lowercase()?.removePrefix("[")?.removeSuffix("]")
            if (host.isNullOrBlank()) return null
            val port = if (parsed.port != -1) parsed.port else parsed.defaultPort.takeIf { it > 0 } ?: 443
            "$host:$port"
        } catch (_: Exception) {
            null
        }
    }

    fun normalizeSha256(raw: String?): String? {
        if (raw.isNullOrBlank()) return null
        val trimmed = raw.trim().removePrefix("sha256/").removePrefix("SHA256/")
        val hexOnly = trimmed.replace(":", "").replace(" ", "").lowercase()
        if (hexOnly.length == 64 && hexOnly.all { it in '0'..'9' || it in 'a'..'f' }) {
            return hexOnly
        }
        val decoded = trimmed.decodeBase64()?.toByteArray()
        if (decoded != null && decoded.size == 32) {
            return decoded.joinToString("") { "%02x".format(it) }
        }
        return null
    }

    fun formatFingerprint(sha256Hex: String): String {
        val norm = normalizeSha256(sha256Hex) ?: return sha256Hex
        return norm.uppercase().chunked(2).joinToString(":")
    }

    fun certificateSha256Hex(cert: X509Certificate): String {
        val digest = MessageDigest.getInstance("SHA-256").digest(cert.encoded)
        return digest.joinToString("") { "%02x".format(it) }
    }

    fun defaultSystemTrustManager(): X509TrustManager? {
        return try {
            val factory = TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm())
            factory.init(null as KeyStore?)
            factory.trustManagers.filterIsInstance<X509TrustManager>().firstOrNull()
        } catch (_: Exception) {
            null
        }
    }

    fun extractX509Certificate(sslCert: SslCertificate?): X509Certificate? {
        if (sslCert == null) return null
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.Q) {
            sslCert.x509Certificate?.let { return it }
        }
        return try {
            val bundle = SslCertificate.saveState(sslCert)
            val bytes = bundle.getByteArray("x509-certificate") ?: return null
            CertificateFactory.getInstance("X.509")
                .generateCertificate(ByteArrayInputStream(bytes)) as? X509Certificate
        } catch (_: Exception) {
            null
        }
    }

    /**
     * Verifies a server certificate chain.
     * - Public addresses strictly require standard CA validation (and honor any explicit pin).
     * - Private/local/Tailscale addresses try standard CA first; if CA validation fails (self-signed cert),
     *   Trust-On-First-Use (TOFU) records and verifies the leaf SHA-256 fingerprint.
     */
    fun verifyServerChain(
        chain: Array<out X509Certificate>?,
        authType: String?,
        hostPort: String,
        isPrivateOrLocal: Boolean,
        pinStore: CertificatePinStore,
        systemTrustManager: X509TrustManager?,
    ) {
        if (chain.isNullOrEmpty()) {
            throw CertificateException("Empty server certificate chain")
        }

        val leaf = chain[0]
        val actualSha256 = certificateSha256Hex(leaf)
        val pinnedSha256 = pinStore.getPin(hostPort)

        if (!isPrivateOrLocal) {
            val tm = systemTrustManager
                ?: throw CertificateException("No system X509TrustManager available")
            tm.checkServerTrusted(chain, authType ?: "RSA")
            if (pinnedSha256 != null && pinnedSha256 != actualSha256) {
                val mismatch = CertificateMismatchException(hostPort, pinnedSha256, actualSha256)
                recordMismatch(mismatch)
                throw mismatch
            }
            return
        }

        // If already pinned (either from QR scan or previous TOFU), enforce pin directly.
        if (pinnedSha256 != null) {
            if (pinnedSha256 == actualSha256) {
                return
            }
            val mismatch = CertificateMismatchException(hostPort, pinnedSha256, actualSha256)
            recordMismatch(mismatch)
            throw mismatch
        }

        // No explicit pin yet: try system CA first (e.g. Tailscale MagicDNS Let's Encrypt certs).
        // Only fall back to TOFU if standard CA verification fails.
        if (systemTrustManager != null) {
            try {
                systemTrustManager.checkServerTrusted(chain, authType ?: "RSA")
                return
            } catch (_: CertificateException) {
                // Expected for self-signed LAN / Tailscale IP certs — proceed to TOFU pinning.
            }
        }

        pinStore.setPin(hostPort, actualSha256)
    }

    fun showMismatchDialog(
        activity: Activity,
        mismatch: CertificateMismatchException,
        onTrustNewCertificate: () -> Unit,
        onCancel: () -> Unit = {},
    ) {
        if (activity.isFinishing || activity.isDestroyed) return
        AlertDialog.Builder(activity)
            .setTitle("Server Certificate Changed")
            .setMessage(
                "The TLS certificate for ${mismatch.hostPort} does not match the previously trusted fingerprint.\n\n" +
                    "Expected:\n${formatFingerprint(mismatch.expectedSha256)}\n\n" +
                    "Presented:\n${formatFingerprint(mismatch.actualSha256)}\n\n" +
                    "Only trust this certificate if you recently regenerated or rotated the server TLS key."
            )
            .setPositiveButton("Trust New Certificate") { _, _ -> onTrustNewCertificate() }
            .setNegativeButton("Cancel") { _, _ -> onCancel() }
            .setOnCancelListener { onCancel() }
            .show()
    }
}

class TofuTrustManager(
    private val hostPort: String,
    private val isPrivateOrLocal: Boolean,
    private val pinStore: CertificatePinStore,
    private val systemTrustManager: X509TrustManager? = TofuPinningSupport.defaultSystemTrustManager(),
) : X509TrustManager {
    override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) {
        systemTrustManager?.checkClientTrusted(chain, authType ?: "RSA")
    }

    override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) {
        TofuPinningSupport.verifyServerChain(
            chain = chain,
            authType = authType,
            hostPort = hostPort,
            isPrivateOrLocal = isPrivateOrLocal,
            pinStore = pinStore,
            systemTrustManager = systemTrustManager,
        )
    }

    override fun getAcceptedIssuers(): Array<X509Certificate> =
        systemTrustManager?.acceptedIssuers ?: arrayOf()
}

class TofuHostnameVerifier(
    private val hostPort: String,
    private val isPrivateOrLocal: Boolean,
    private val pinStore: CertificatePinStore,
    private val delegate: HostnameVerifier = OkHostnameVerifier,
) : HostnameVerifier {
    override fun verify(hostname: String?, session: SSLSession?): Boolean {
        if (hostname.isNullOrBlank() || session == null) return false
        if (delegate.verify(hostname, session)) return true
        if (!isPrivateOrLocal) return false

        val pinned = pinStore.getPin(hostPort) ?: return false
        val leaf = try {
            session.peerCertificates.firstOrNull() as? X509Certificate
        } catch (_: SSLPeerUnverifiedException) {
            null
        } ?: return false

        val actual = TofuPinningSupport.certificateSha256Hex(leaf)
        return actual == pinned
    }
}
