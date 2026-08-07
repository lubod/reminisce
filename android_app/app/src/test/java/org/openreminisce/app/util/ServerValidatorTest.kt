package org.openreminisce.app.util

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Seed test suite for ServerValidator's pure URL validation logic
 * (no Android framework dependencies).
 */
class ServerValidatorTest {

    @Test
    fun validUrlsAreAccepted() {
        assertTrue(ServerValidator.isValidUrl("http://192.168.1.55:11111"))
        assertTrue(ServerValidator.isValidUrl("https://dnet.example.com:8443"))
        assertTrue(ServerValidator.isValidUrl("http://localhost:8080/api"))
    }

    @Test
    fun surroundingWhitespaceIsIgnored() {
        assertTrue(ServerValidator.isValidUrl("  https://dnet.example.com:8443  "))
    }

    @Test
    fun invalidUrlsAreRejected() {
        assertFalse(ServerValidator.isValidUrl(""))
        assertFalse(ServerValidator.isValidUrl("ftp://host"))
        assertFalse(ServerValidator.isValidUrl("192.168.1.55:11111"))
        assertFalse(ServerValidator.isValidUrl("not a url"))
        assertFalse(ServerValidator.isValidUrl("http://"))
    }
}
