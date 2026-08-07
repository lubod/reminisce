package org.openreminisce.app.service

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Seed test suite for MIME-type guessing used by uploads.
 */
class ProgressRequestBodyTest {

    @Test
    fun mapsImageExtensionsToJpeg() {
        assertEquals("image/jpeg", ProgressRequestBody.guessMimeType("IMG_20240101_123456.jpg"))
        assertEquals("image/jpeg", ProgressRequestBody.guessMimeType("photo.JPEG"))
        assertEquals("image/jpeg", ProgressRequestBody.guessMimeType("scan.jpeg"))
    }

    @Test
    fun mapsOtherKnownExtensions() {
        assertEquals("image/png", ProgressRequestBody.guessMimeType("screenshot.png"))
        assertEquals("image/gif", ProgressRequestBody.guessMimeType("anim.gif"))
        assertEquals("video/mp4", ProgressRequestBody.guessMimeType("movie.mp4"))
        assertEquals("video/quicktime", ProgressRequestBody.guessMimeType("movie.mov"))
        assertEquals("video/x-msvideo", ProgressRequestBody.guessMimeType("movie.avi"))
        assertEquals("video/x-matroska", ProgressRequestBody.guessMimeType("movie.mkv"))
    }

    @Test
    fun unknownExtensionsFallBackToOctetStream() {
        assertEquals("application/octet-stream", ProgressRequestBody.guessMimeType("noext"))
        assertEquals("application/octet-stream", ProgressRequestBody.guessMimeType("archive.zip"))
    }
}
