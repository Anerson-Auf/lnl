package su.yufu.lnl.data

import java.io.IOException
import java.nio.charset.StandardCharsets
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import okhttp3.Call
import okhttp3.Callback
import okhttp3.HttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import su.yufu.lnl.BuildConfig
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

class RelayApiException(message: String) : IOException(message)

interface RelayClient {
    val normalizedBaseUrl: String
    suspend fun sessions(): List<SessionSummary>
    suspend fun chats(sessionId: String): List<ChatSummary>
    suspend fun messages(sessionId: String, peerId: Long): List<Message>
    suspend fun send(sessionId: String, peerId: Long, text: String): SendResponse
    fun events(sessionId: String): Flow<SocketEvent>
}

class LnlApi(
    baseUrl: String,
    allowCleartext: Boolean = BuildConfig.ALLOW_CLEARTEXT,
) : RelayClient {
    private val base: HttpUrl = normalizeBaseUrl(baseUrl, allowCleartext)
    override val normalizedBaseUrl: String = base.toString().trimEnd('/')
    private val json = Json {
        ignoreUnknownKeys = true
    }
    private val jsonMediaType = "application/json; charset=utf-8".toMediaType()

    override suspend fun sessions(): List<SessionSummary> = get(url("api", "sessions"))

    override suspend fun chats(sessionId: String): List<ChatSummary> =
        get(scopedUrl(sessionId, "chats"))

    override suspend fun messages(sessionId: String, peerId: Long): List<Message> =
        get(scopedUrl(sessionId, "messages", peerId.toString()))

    override suspend fun send(sessionId: String, peerId: Long, text: String): SendResponse {
        val requestBody = json.encodeToString(SendBody(text)).toRequestBody(jsonMediaType)
        val request = Request.Builder()
            .url(scopedUrl(sessionId, "messages", peerId.toString()))
            .post(requestBody)
            .build()
        return execute(request)
    }

    override fun events(sessionId: String): Flow<SocketEvent> = callbackFlow {
        val request = Request.Builder()
            .url(webSocketUrl(sessionId))
            .build()
        val socket = httpClient.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                trySend(SocketEvent.Connected)
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                if (text.length > MAX_WS_EVENT_BYTES ||
                    text.toByteArray(StandardCharsets.UTF_8).size > MAX_WS_EVENT_BYTES
                ) {
                    webSocket.close(1009, "Event too large")
                    close(RelayApiException("Событие сервера слишком большое"))
                    return
                }
                runCatching { json.decodeFromString<WsEvent>(text) }
                    .onSuccess { trySend(SocketEvent.Payload(it)) }
            }

            override fun onFailure(webSocket: WebSocket, throwable: Throwable, response: Response?) {
                close(throwable)
            }

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                webSocket.close(code, reason)
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                close()
            }
        })
        awaitClose { socket.cancel() }
    }

    private fun scopedUrl(sessionId: String, vararg segments: String): HttpUrl =
        url("api", "sessions", sessionId, *segments)

    private fun webSocketUrl(sessionId: String): HttpUrl {
        return scopedUrl(sessionId, "ws")
    }

    private fun url(vararg segments: String): HttpUrl {
        val builder = base.newBuilder()
        segments.forEach(builder::addPathSegment)
        return builder.build()
    }

    private suspend inline fun <reified T> get(url: HttpUrl): T {
        val request = Request.Builder().url(url).get().build()
        return execute(request)
    }

    private suspend inline fun <reified T> execute(request: Request): T =
        await(request) { response ->
            response.use {
                val raw = response.readBodyLimited()
                if (!response.isSuccessful) {
                    val body = runCatching { json.decodeFromString<ErrorBody>(raw) }.getOrNull()
                    throw RelayApiException(body?.error ?: "HTTP ${response.code}")
                }
                runCatching { json.decodeFromString<T>(raw) }
                    .getOrElse { throw RelayApiException("Сервер вернул некорректный JSON") }
            }
        }

    private suspend fun <T> await(
        request: Request,
        transform: (Response) -> T,
    ): T = suspendCancellableCoroutine { continuation ->
        val call = httpClient.newCall(request)
        continuation.invokeOnCancellation { call.cancel() }
        call.enqueue(object : Callback {
            override fun onFailure(call: Call, failure: IOException) {
                if (continuation.isActive) {
                    continuation.resumeWithException(failure)
                }
            }

            override fun onResponse(call: Call, response: Response) {
                if (!continuation.isActive) {
                    response.close()
                    return
                }
                runCatching { transform(response) }
                    .onSuccess { result ->
                        if (continuation.isActive) continuation.resume(result)
                    }
                    .onFailure { failure ->
                        if (continuation.isActive) continuation.resumeWithException(failure)
                    }
            }
        })
    }

    private fun Response.readBodyLimited(): String {
        val responseBody = body ?: return ""
        val declaredLength = responseBody.contentLength()
        if (declaredLength > MAX_JSON_BYTES) {
            throw RelayApiException("Ответ сервера слишком большой")
        }
        val source = responseBody.source()
        source.request(MAX_JSON_BYTES + 1)
        if (source.buffer.size > MAX_JSON_BYTES) {
            throw RelayApiException("Ответ сервера слишком большой")
        }
        return source.buffer.clone().readString(StandardCharsets.UTF_8)
    }

    private companion object {
        const val MAX_JSON_BYTES = 4L * 1024 * 1024
        const val MAX_WS_EVENT_BYTES = 256 * 1024

        val httpClient: OkHttpClient = OkHttpClient.Builder()
            .connectTimeout(15, TimeUnit.SECONDS)
            .readTimeout(30, TimeUnit.SECONDS)
            .build()

        fun normalizeBaseUrl(raw: String, allowCleartext: Boolean): HttpUrl {
            val parsed = raw.trim().toHttpUrlOrNull()
                ?: throw IllegalArgumentException("Укажи корректный Relay URL")
            require(parsed.scheme == "http" || parsed.scheme == "https") {
                "Relay URL должен использовать http или https"
            }
            require(allowCleartext || parsed.isHttps) {
                "В release-сборке Relay URL должен использовать https"
            }
            require(parsed.username.isEmpty() && parsed.password.isEmpty()) {
                "Логин и пароль нельзя помещать в Relay URL"
            }
            require(parsed.query == null && parsed.fragment == null) {
                "Relay URL не должен содержать query или fragment"
            }
            return parsed
        }
    }
}
