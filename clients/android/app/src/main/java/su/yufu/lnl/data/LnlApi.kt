package su.yufu.lnl.data

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.withContext
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import java.util.concurrent.TimeUnit

class LnlApi(baseUrl: String) {
    private val base = baseUrl.trimEnd('/')
    private val json = Json {
        ignoreUnknownKeys = true
        isLenient = true
    }
    private val client = OkHttpClient.Builder()
        .connectTimeout(15, TimeUnit.SECONDS)
        .readTimeout(30, TimeUnit.SECONDS)
        .build()
    private val media = "application/json; charset=utf-8".toMediaType()

    suspend fun chats(): List<ChatSummary> = get("/api/chats")

    suspend fun messages(peerId: Long): List<Message> = get("/api/messages/$peerId")

    suspend fun send(peerId: Long, text: String): SendResponse {
        val body = json.encodeToString(SendBody(text))
        return post("/api/messages/$peerId", body)
    }

    fun events(): Flow<WsEvent> = callbackFlow {
        val wsUrl = base
            .replace("https://", "wss://")
            .replace("http://", "ws://") + "/ws"
        val request = Request.Builder().url(wsUrl).build()
        val ws = client.newWebSocket(request, object : WebSocketListener() {
            override fun onMessage(webSocket: WebSocket, text: String) {
                runCatching { json.decodeFromString<WsEvent>(text) }
                    .onSuccess { trySend(it) }
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                close(t)
            }

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                webSocket.close(code, reason)
                close()
            }
        })
        awaitClose { ws.cancel() }
    }

    private suspend inline fun <reified T> get(path: String): T = withContext(Dispatchers.IO) {
        val req = Request.Builder().url(base + path).get().build()
        client.newCall(req).execute().use { resp ->
            val raw = resp.body?.string().orEmpty()
            if (!resp.isSuccessful) {
                val err = runCatching { json.decodeFromString<ErrorBody>(raw) }.getOrNull()
                error(err?.error ?: "HTTP ${resp.code}")
            }
            json.decodeFromString(raw)
        }
    }

    private suspend inline fun <reified T> post(path: String, body: String): T =
        withContext(Dispatchers.IO) {
            val req = Request.Builder()
                .url(base + path)
                .post(body.toRequestBody(media))
                .build()
            client.newCall(req).execute().use { resp ->
                val raw = resp.body?.string().orEmpty()
                if (!resp.isSuccessful) {
                    val err = runCatching { json.decodeFromString<ErrorBody>(raw) }.getOrNull()
                    error(err?.error ?: "HTTP ${resp.code}")
                }
                json.decodeFromString(raw)
            }
        }
}
