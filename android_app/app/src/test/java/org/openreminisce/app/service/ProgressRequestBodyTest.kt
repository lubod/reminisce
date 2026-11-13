package org.openreminisce.app.service

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Unit tests for MIME-type guessing used by uploads.
 */
class ProgressRequestBodyTest {

    @Test
    fun mapsImageExtensionsProperly() {
        assertEquals("image/jpeg", ProgressRequestBody.guessMimeType("IMG_20240101_123456.jpg"))
        assertEquals("image/jpeg", ProgressRequestBody.guessMimeType("photo.JPEG"))
        assertEquals("image/jpeg", ProgressRequestBody.guessMimeType("scan.jpeg"))
        assertEquals("image/png", ProgressRequestBody.guessMimeType("screenshot.PNG"))
        assertEquals("image/gif", ProgressRequestBody.guessMimeType("anim.gif"))
        assertEquals("image/webp", ProgressRequestBody.guessMimeType("sticker.WEBP"))
        assertEquals("image/heic", ProgressRequestBody.guessMimeType("iphone.heic"))
        assertEquals("image/heif", ProgressRequestBody.guessMimeType("frame.HEIF"))
        assertEquals("image/avif", ProgressRequestBody.guessMimeType("modern.avif"))
        assertEquals("image/bmp", ProgressRequestBody.guessMimeType("raw.bmp"))
        assertEquals("image/tiff", ProgressRequestBody.guessMimeType("scan.tif"))
        assertEquals("image/tiff", ProgressRequestBody.guessMimeType("scan.TIFF"))
        assertEquals("image/x-adobe-dng", ProgressRequestBody.guessMimeType("capture.dng"))
    }

    @Test
    fun mapsVideoExtensionsProperly() {
        assertEquals("video/mp4", ProgressRequestBody.guessMimeType("movie.mp4"))
        assertEquals("video/mp4", ProgressRequestBody.guessMimeType("clip.M4V"))
        assertEquals("video/quicktime", ProgressRequestBody.guessMimeType("movie.MOV"))
        assertEquals("video/x-msvideo", ProgressRequestBody.guessMimeType("movie.avi"))
        assertEquals("video/x-matroska", ProgressRequestBody.guessMimeType("movie.mkv"))
        assertEquals("video/webm", ProgressRequestBody.guessMimeType("web.webm"))
        assertEquals("video/3gpp", ProgressRequestBody.guessMimeType("legacy.3gp"))
        assertEquals("video/mp2t", ProgressRequestBody.guessMimeType("camcorder.mts"))
        assertEquals("video/mp2t", ProgressRequestBody.guessMimeType("camcorder.M2TS"))
    }

    @Test
    fun unknownOrTrailingDotFallsBackToOctetStream() {
        assertEquals("application/octet-stream", ProgressRequestBody.guessMimeType("noext"))
        assertEquals("application/octet-stream", ProgressRequestBody.guessMimeType("archive.zip"))
        assertEquals("application/octet-stream", ProgressRequestBody.guessMimeType("trailing."))
    }
}
