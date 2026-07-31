package su.yufu.lnl.data

import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

class LnlApiTest {
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
    fun usesScopedRoutesAndPreservesBasePath() = runBlocking {
        val api = LnlApi(server.url("/relay/").toString())
        server.enqueue(
            MockResponse().setBody("""[{"id":"work_team","is_default":true}]"""),
        )
        server.enqueue(
            MockResponse().setBody("""[{"peer_id":42,"title":"Chat","pinned":true}]"""),
        )

        val sessions = api.sessions()
        val chats = api.chats(sessions.single().id)

        assertEquals("work_team", sessions.single().id)
        assertTrue(chats.single().pinned)
        assertEquals("/relay/api/sessions", server.takeRequest().path)
        assertEquals(
            "/relay/api/sessions/work_team/chats",
            server.takeRequest().path,
        )
    }

    @Test
    fun sendsTextOnlyToTheSelectedSession() = runBlocking {
        val api = LnlApi(server.url("/").toString())
        server.enqueue(
            MockResponse().setBody(
                """{
                    "ok":true,
                    "message":{"id":9,"text":"hello","outgoing":true,"date":1}
                }""".trimIndent(),
            ),
        )

        val response = api.send("work", 123, "hello")
        val request = server.takeRequest()

        assertTrue(response.ok)
        assertEquals("/api/sessions/work/messages/123", request.path)
        assertEquals("""{"text":"hello"}""", request.body.readUtf8())
    }

    @Test
    fun surfacesServerErrors() {
        val api = LnlApi(server.url("/").toString())
        server.enqueue(
            MockResponse()
                .setResponseCode(503)
                .setBody("""{"error":"аккаунт ещё не авторизован"}"""),
        )

        val failure = assertThrows(RelayApiException::class.java) {
            runBlocking { api.chats("work") }
        }

        assertEquals("аккаунт ещё не авторизован", failure.message)
    }

    @Test
    fun rejectsUnsafeRelayUrls() {
        listOf(
            "http://user:password@example.test",
            "https://example.test?token=value",
            "https://example.test/#fragment",
        ).forEach { url ->
            assertThrows(IllegalArgumentException::class.java) {
                LnlApi(url)
            }
        }
        assertThrows(IllegalArgumentException::class.java) {
            LnlApi("http://example.test", allowCleartext = false)
        }
        assertEquals(
            "https://example.test",
            LnlApi("https://example.test/", allowCleartext = false).normalizedBaseUrl,
        )
    }

    @Test
    fun rejectsOversizedDeclaredAndChunkedBodies() {
        val api = LnlApi(server.url("/").toString())
        val oversized = "x".repeat(4 * 1024 * 1024 + 1)
        server.enqueue(MockResponse().setBody(oversized))
        server.enqueue(MockResponse().setChunkedBody(oversized, 8 * 1024))

        repeat(2) {
            val failure = assertThrows(RelayApiException::class.java) {
                runBlocking { api.sessions() }
            }
            assertEquals("Ответ сервера слишком большой", failure.message)
        }
    }

    @Test
    fun opensScopedWebSocketUnderBasePath() = runBlocking {
        server.enqueue(
            MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
                override fun onOpen(webSocket: WebSocket, response: Response) = Unit
            }),
        )
        val api = LnlApi(server.url("/relay/").toString())

        withTimeout(2_000) {
            api.events("work_team").first()
        }

        assertEquals(
            "/relay/api/sessions/work_team/ws",
            server.takeRequest().path,
        )
    }
}
