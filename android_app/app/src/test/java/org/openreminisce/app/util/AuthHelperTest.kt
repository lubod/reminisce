package org.openreminisce.app.util

import okhttp3.OkHttpClient
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okio.ByteString.Companion.encodeUtf8
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import java.util.concurrent.Callable
import java.util.concurrent.Executors

class AuthHelperTest {

    private lateinit var server: MockWebServer

    @Before
    fun setUp() {
        server = MockWebServer()
        server.start()
    }

    @After
    fun tearDown() {
        server.shutdown()
    }

    @Test
    fun isTokenValidChecksJwtExpiryAcrossPaddingCases() {
        val futureExp = (System.currentTimeMillis() / 1000L) + 3600L
        val pastExp = (System.currentTimeMillis() / 1000L) - 60L

        val validJwt = buildJwt("""{"sub":"1","exp":$futureExp}""")
        val validUnalignedJwt = buildJwt("""{"sub":"12345","exp":$futureExp}""")
        val expiredJwt = buildJwt("""{"sub":"1","exp":$pastExp}""")

        assertTrue(AuthHelper.isTokenValid(validJwt))
        assertTrue(AuthHelper.isTokenValid(validUnalignedJwt))
        assertFalse(AuthHelper.isTokenValid(expiredJwt))
        assertFalse(AuthHelper.isTokenValid("not-a-jwt"))
        assertFalse(AuthHelper.isTokenValid("a.b"))
    }

    @Test
    fun performTokenRefreshRotatesTokensOnSuccess() {
        val futureExp = (System.currentTimeMillis() / 1000L) + 3600L
        val rotatedAccess = buildJwt("""{"sub":"1","exp":$futureExp}""")
        server.enqueue(
            MockResponse()
                .setResponseCode(200)
                .setBody("""{"access_token":"$rotatedAccess","refresh_token":"refresh-2"}""")
        )

        val store = FakeTokenPairStore(access = null, refresh = "refresh-1")
        val client = OkHttpClient()
        val result = AuthHelper.performTokenRefresh(
            client = client,
            baseUrl = server.url("/").toString(),
            deviceId = "dev-42",
            tokenStore = store
        )

        assertEquals(rotatedAccess, result)
        assertEquals(rotatedAccess, store.access)
        assertEquals("refresh-2", store.refresh)

        val recorded = server.takeRequest()
        assertEquals("/api/auth/refresh", recorded.path)
        assertTrue(recorded.body.readUtf8().contains("refresh-1"))
    }

    @Test
    fun performTokenRefreshClearsCredentialsOn401ButPreservesOn500() {
        val store401 = FakeTokenPairStore(access = "old", refresh = "refresh-bad")
        server.enqueue(MockResponse().setResponseCode(401))
        val res401 = AuthHelper.performTokenRefresh(
            OkHttpClient(),
            server.url("/").toString(),
            "dev-1",
            store401
        )
        assertNull(res401)
        assertTrue(store401.cleared)
        assertNull(store401.refresh)

        val store500 = FakeTokenPairStore(access = "old", refresh = "refresh-keep")
        server.enqueue(MockResponse().setResponseCode(500))
        val res500 = AuthHelper.performTokenRefresh(
            OkHttpClient(),
            server.url("/").toString(),
            "dev-1",
            store500
        )
        assertNull(res500)
        assertFalse(store500.cleared)
        assertEquals("refresh-keep", store500.refresh)
    }

    @Test
    fun concurrentRefreshCallsShareSingleServerRotation() {
        val futureExp = (System.currentTimeMillis() / 1000L) + 3600L
        val staleAccess = buildJwt("""{"sub":"1","exp":$futureExp}""")
        val newAccess = buildJwt("""{"sub":"2","exp":${futureExp + 60}}""")

        server.enqueue(
            MockResponse()
                .setResponseCode(200)
                .setBody("""{"access_token":"$newAccess","refresh_token":"refresh-rotated"}""")
        )

        val store = FakeTokenPairStore(access = staleAccess, refresh = "refresh-initial")
        val client = OkHttpClient()
        val pool = Executors.newFixedThreadPool(2)
        val futures = listOf(
            pool.submit(Callable {
                AuthHelper.performTokenRefresh(client, server.url("/").toString(), "dev-1", store, failedAccessToken = staleAccess)
            }),
            pool.submit(Callable {
                AuthHelper.performTokenRefresh(client, server.url("/").toString(), "dev-1", store, failedAccessToken = staleAccess)
            })
        )

        assertEquals(newAccess, futures[0].get())
        assertEquals(newAccess, futures[1].get())
        assertEquals(1, server.requestCount)
        pool.shutdown()
    }

    private fun buildJwt(payloadJson: String): String {
        val header = """{"alg":"HS256","typ":"JWT"}""".encodeUtf8().base64Url().trimEnd('=')
        val payload = payloadJson.encodeUtf8().base64Url().trimEnd('=')
        return "$header.$payload.sig"
    }

    private class FakeTokenPairStore(
        var access: String?,
        var refresh: String?
    ) : TokenPairStore {
        var cleared = false
        override fun getAccessToken(): String? = access
        override fun setAccessToken(token: String) {
            access = token
        }
        override fun getRefreshToken(): String? = refresh
        override fun setRefreshToken(token: String) {
            refresh = token
        }
        override fun clearCredentials() {
            cleared = true
            access = null
            refresh = null
        }
    }
}
