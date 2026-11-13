package org.openreminisce.app.repository

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class RemoteMediaRepositoryParseTest {

    @Test
    fun parseThumbnailResponseHandlesNullableFieldsAndMetadata() {
        val json = """
            {
              "thumbnails": [
                {
                  "hash": "h1",
                  "created_at": "2026-01-01T00:00:00Z",
                  "place": "null",
                  "media_type": "image",
                  "name": "img1.jpg",
                  "starred": true,
                  "device_id": ""
                },
                {
                  "hash": "h2",
                  "created_at": "2026-01-02T00:00:00Z",
                  "place": "Zurich",
                  "media_type": "video",
                  "name": "vid2.mp4",
                  "starred": false,
                  "device_id": "pixel-8"
                }
              ],
              "total": 42,
              "page": 2,
              "limit": 50
            }
        """.trimIndent()

        val parsed = RemoteMediaRepository.parseThumbnailResponse(json)
        assertEquals(42, parsed.total)
        assertEquals(2, parsed.page)
        assertEquals(50, parsed.limit)
        assertEquals(2, parsed.thumbnails.size)

        assertEquals("h1", parsed.thumbnails[0].hash)
        assertNull(parsed.thumbnails[0].place)
        assertNull(parsed.thumbnails[0].device_id)
        assertTrue(parsed.thumbnails[0].starred)

        assertEquals("Zurich", parsed.thumbnails[1].place)
        assertEquals("video", parsed.thumbnails[1].mediaType)
        assertEquals("pixel-8", parsed.thumbnails[1].device_id)
    }

    @Test
    fun parseSearchResponseReadsSimilarityAndEmptyResults() {
        val json = """
            {
              "results": [
                {
                  "hash": "s1",
                  "created_at": "2026-02-01T12:00:00Z",
                  "place": "Alps",
                  "media_type": "image",
                  "name": "peak.jpg",
                  "starred": false,
                  "similarity": 0.875
                }
              ],
              "total": 1
            }
        """.trimIndent()

        val parsed = RemoteMediaRepository.parseSearchResponse(json)
        assertEquals(1, parsed.total)
        assertEquals(1, parsed.page)
        assertEquals(1, parsed.thumbnails.size)
        assertEquals("Alps", parsed.thumbnails[0].place)
        assertEquals(0.875f, parsed.thumbnails[0].similarity ?: 0f, 0.0001f)

        val empty = RemoteMediaRepository.parseSearchResponse("""{"results": []}""")
        assertEquals(0, empty.total)
        assertTrue(empty.thumbnails.isEmpty())
    }
}
