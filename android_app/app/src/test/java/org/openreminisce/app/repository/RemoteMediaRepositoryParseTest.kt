package org.openreminisce.app.repository

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.openreminisce.app.model.MediaFilter
import org.openreminisce.app.model.MediaTypeFilter

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
                  "device_id": "",
                  "thumbnail_url": "/api/thumbnail/h1?o=6",
                  "file_size_bytes": 123456,
                  "aesthetic_score": 7.8,
                  "orientation": 6,
                  "distance_km": 4.2
                },
                {
                  "hash": "h2",
                  "created_at": "2026-01-02T00:00:00Z",
                  "place": "Zurich",
                  "media_type": "video",
                  "name": "vid2.mp4",
                  "starred": false,
                  "device_id": "pixel-8",
                  "file_size_bytes": null,
                  "aesthetic_score": null,
                  "orientation": null
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

        val first = parsed.thumbnails[0]
        assertEquals("h1", first.hash)
        assertNull(first.place)
        assertNull(first.device_id)
        assertTrue(first.starred)
        assertEquals("/api/thumbnail/h1?o=6", first.thumbnailUrl)
        assertEquals(123456L, first.fileSizeBytes)
        assertEquals(7.8f, first.aestheticScore ?: 0f, 0.001f)
        assertEquals(6, first.orientation)
        assertEquals(4.2f, first.distance_km ?: 0f, 0.001f)

        val second = parsed.thumbnails[1]
        assertEquals("Zurich", second.place)
        assertEquals("video", second.mediaType)
        assertEquals("pixel-8", second.device_id)
        assertNull(second.thumbnailUrl)
        assertNull(second.fileSizeBytes)
        assertNull(second.aestheticScore)
        assertNull(second.orientation)
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
                  "similarity": 0.875,
                  "thumbnail_url": "/api/thumbnail/s1?o=1"
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
        assertEquals("/api/thumbnail/s1?o=1", parsed.thumbnails[0].thumbnailUrl)

        val empty = RemoteMediaRepository.parseSearchResponse("""{"results": []}""")
        assertEquals(0, empty.total)
        assertTrue(empty.thumbnails.isEmpty())
    }

    @Test
    fun parseDeviceIdsResponseHandlesObjectAndLegacyArray() {
        val objectJson = """{"device_ids": ["pixel-8", "ipad", "", "null"]}"""
        assertEquals(listOf("pixel-8", "ipad"), RemoteMediaRepository.parseDeviceIdsResponse(objectJson))

        val arrayJson = """["pixel-8"]"""
        assertEquals(listOf("pixel-8"), RemoteMediaRepository.parseDeviceIdsResponse(arrayJson))

        val emptyJson = """{"device_ids": []}"""
        assertTrue(RemoteMediaRepository.parseDeviceIdsResponse(emptyJson).isEmpty())
    }

    @Test
    fun parsePlacesResponseHandlesBackendArrayLegacyWrapperAndInvalidCoords() {
        val backendJson = """
            [
              {
                "name": "Paris",
                "display_name": "Paris, Ile-de-France, FR",
                "latitude": 48.8566,
                "longitude": 2.3522,
                "admin_level": 8,
                "country_code": "FR"
              },
              {
                "name": "BadLat",
                "display_name": "Invalid",
                "latitude": 120.0,
                "longitude": 2.0
              }
            ]
        """.trimIndent()
        val backendParsed = RemoteMediaRepository.parsePlacesResponse(backendJson)
        assertEquals(1, backendParsed.size)
        assertEquals("Paris", backendParsed[0].name)
        assertEquals("Paris, Ile-de-France, FR", backendParsed[0].display_name)
        assertEquals(48.8566, backendParsed[0].latitude, 0.0001)
        assertEquals(2.3522, backendParsed[0].longitude, 0.0001)
        assertEquals("FR", backendParsed[0].country_code)

        val legacyJson = """
            {
              "places": [
                {
                  "name": "Rome",
                  "lat": 41.9,
                  "lon": 12.5
                }
              ]
            }
        """.trimIndent()
        val legacyParsed = RemoteMediaRepository.parsePlacesResponse(legacyJson)
        assertEquals(1, legacyParsed.size)
        assertEquals("Rome", legacyParsed[0].name)
        assertEquals("Rome", legacyParsed[0].display_name)
        assertEquals(41.9, legacyParsed[0].latitude, 0.0001)
        assertEquals(12.5, legacyParsed[0].longitude, 0.0001)
    }

    @Test
    fun mediaFilterActiveFilterCountTracksNonDefaultFields() {
        val defaultFilter = MediaFilter()
        assertTrue(defaultFilter.isDefault())
        assertEquals(0, defaultFilter.activeFilterCount())

        val partial = defaultFilter.copy(
            mediaType = MediaTypeFilter.IMAGE,
            starredOnly = true,
            labelId = 7
        )
        assertEquals(3, partial.activeFilterCount())

        val full = MediaFilter(
            mediaType = MediaTypeFilter.VIDEO,
            starredOnly = true,
            startDate = 1000L,
            labelId = 5,
            deviceId = "pixel-8",
            locationLat = 48.85,
            locationLon = 2.35,
            sortBy = "quality",
            sortOrder = "asc",
            groupByPlace = true
        )
        assertEquals(8, full.activeFilterCount())
    }

    @Test
    fun parseMapPointsResponseParsesPointsAndSkipsInvalidEntries() {
        val json = """
            {
              "points": [
                {
                  "hash": "abc123",
                  "lon": 2.3522,
                  "lat": 48.8566,
                  "created_at": "2026-03-01T10:15:00Z",
                  "place": "Paris, France",
                  "starred": true,
                  "device_id": "pixel-8",
                  "has_thumbnail": true
                },
                {
                  "hash": "def456",
                  "lon": 8.5417,
                  "lat": 47.3769,
                  "created_at": "2026-03-02T12:00:00Z",
                  "place": "null",
                  "starred": false,
                  "device_id": "",
                  "has_thumbnail": false
                },
                {
                  "hash": "",
                  "lon": 0.0,
                  "lat": 0.0
                }
              ],
              "total": 250
            }
        """.trimIndent()

        val parsed = RemoteMediaRepository.parseMapPointsResponse(json)
        assertEquals(250, parsed.total)
        assertEquals(2, parsed.points.size)

        assertEquals("abc123", parsed.points[0].hash)
        assertEquals(48.8566, parsed.points[0].lat, 0.0001)
        assertEquals(2.3522, parsed.points[0].lon, 0.0001)
        assertEquals("Paris, France", parsed.points[0].place)
        assertTrue(parsed.points[0].starred)
        assertEquals("pixel-8", parsed.points[0].device_id)
        assertTrue(parsed.points[0].has_thumbnail)

        assertEquals("def456", parsed.points[1].hash)
        assertNull(parsed.points[1].place)
        assertNull(parsed.points[1].device_id)
        assertFalse(parsed.points[1].starred)
        assertFalse(parsed.points[1].has_thumbnail)
    }

    @Test
    fun parseUpdatePlaceResponseHandlesValueAndClearedNull() {
        val setJson = """{"status": "success", "place": "Zermatt, Switzerland", "latitude": 46.0207, "longitude": 7.7491}"""
        assertEquals("Zermatt, Switzerland", RemoteMediaRepository.parseUpdatePlaceResponse(setJson))

        val clearedJson = """{"status": "success", "place": null}"""
        assertNull(RemoteMediaRepository.parseUpdatePlaceResponse(clearedJson))

        val emptyJson = """{"status": "success", "place": ""}"""
        assertNull(RemoteMediaRepository.parseUpdatePlaceResponse(emptyJson))
    }
}
