package org.openreminisce.app.rust

import org.bouncycastle.crypto.digests.Blake3Digest

/**
 * Pure-Java BLAKE3 hasher using BouncyCastle.
 * Drop-in replacement for the old UniFFI Rust Blake3Hasher.
 */
class Blake3Hasher {
    private val digest = Blake3Digest(256)

    fun update(data: ByteArray) {
        digest.update(data, 0, data.size)
    }

    fun finalize(): String {
        val hash = ByteArray(32)
        digest.doFinal(hash, 0)
        return hash.joinToString("") { "%02x".format(it) }
    }
}
