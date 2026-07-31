package su.yufu.lnl.ui

import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import java.util.Locale
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import su.yufu.lnl.data.ChatSummary
import su.yufu.lnl.data.LnlApi
import su.yufu.lnl.data.Message
import su.yufu.lnl.data.RelayClient
import su.yufu.lnl.data.RelaySettingsStore
import su.yufu.lnl.data.SessionSummary
import su.yufu.lnl.data.SocketEvent
import su.yufu.lnl.data.WsEvent

private const val MAX_RETAINED_MESSAGES = 500
private const val STABLE_CONNECTION_NANOS = 30_000_000_000L

enum class ConnectionStatus {
    Idle,
    Connecting,
    Online,
    Offline,
}

data class LnlUiState(
    val baseUrlDraft: String,
    val baseUrl: String,
    val sessions: List<SessionSummary> = emptyList(),
    val selectedSessionId: String? = null,
    val chats: List<ChatSummary> = emptyList(),
    val activeChat: ChatSummary? = null,
    val messages: List<Message> = emptyList(),
    val draft: String = "",
    val connection: ConnectionStatus = ConnectionStatus.Idle,
    val loadingSessions: Boolean = false,
    val loadingChats: Boolean = false,
    val loadingMessages: Boolean = false,
    val sending: Boolean = false,
    val messagesError: String? = null,
    val errorMessage: String? = null,
)

class LnlViewModel(
    private val preferences: RelaySettingsStore,
    private val apiFactory: (String) -> RelayClient = { LnlApi(it) },
) : ViewModel() {
    private val savedSettings = preferences.load()
    private val _state = MutableStateFlow(
        LnlUiState(
            baseUrlDraft = savedSettings.baseUrl,
            baseUrl = savedSettings.baseUrl,
        ),
    )
    val state = _state.asStateFlow()

    private var api: RelayClient? = null
    private var connectJob: Job? = null
    private var chatsJob: Job? = null
    private var messagesJob: Job? = null
    private var sendJob: Job? = null
    private var socketJob: Job? = null
    private var chatRevision = 0L
    private var foreground = false

    init {
        if (savedSettings.baseUrl.isNotBlank()) {
            connect(savedSettings.baseUrl, savedSettings.sessionId)
        }
    }

    fun updateBaseUrlDraft(value: String) {
        _state.update { it.copy(baseUrlDraft = value) }
    }

    fun connect() {
        connect(_state.value.baseUrlDraft, _state.value.selectedSessionId)
    }

    fun setForeground(value: Boolean) {
        if (foreground == value) return
        foreground = value
        if (!value) {
            socketJob?.cancel()
            socketJob = null
            _state.update { it.copy(connection = ConnectionStatus.Idle) }
            return
        }
        val currentApi = api ?: return
        val sessionId = _state.value.selectedSessionId ?: return
        observeEvents(currentApi, sessionId)
    }

    fun selectSession(sessionId: String) {
        val current = _state.value
        val currentApi = api ?: return
        if (current.selectedSessionId == sessionId || current.sessions.none { it.id == sessionId }) {
            return
        }
        preferences.saveSessionId(sessionId)
        chatsJob?.cancel()
        messagesJob?.cancel()
        sendJob?.cancel()
        socketJob?.cancel()
        chatRevision = 0
        _state.update {
            it.copy(
                selectedSessionId = sessionId,
                chats = emptyList(),
                activeChat = null,
                messages = emptyList(),
                draft = "",
                sending = false,
                messagesError = null,
                connection = if (foreground) {
                    ConnectionStatus.Connecting
                } else {
                    ConnectionStatus.Idle
                },
                errorMessage = null,
            )
        }
        refreshChats(currentApi, sessionId)
        observeEvents(currentApi, sessionId)
    }

    fun refreshChats() {
        val currentApi = api ?: return
        val sessionId = _state.value.selectedSessionId ?: return
        refreshChats(currentApi, sessionId)
    }

    fun openChat(chat: ChatSummary) {
        val currentApi = api ?: return
        val sessionId = _state.value.selectedSessionId ?: return
        sendJob?.cancel()
        _state.update {
            it.copy(
                activeChat = chat,
                messages = emptyList(),
                draft = "",
                loadingMessages = true,
                sending = false,
                messagesError = null,
                errorMessage = null,
            )
        }
        loadMessages(currentApi, sessionId, chat, clearExisting = true)
    }

    fun retryMessages() {
        val currentApi = api ?: return
        val sessionId = _state.value.selectedSessionId ?: return
        val chat = _state.value.activeChat ?: return
        loadMessages(currentApi, sessionId, chat, clearExisting = false)
    }

    fun closeChat() {
        messagesJob?.cancel()
        sendJob?.cancel()
        _state.update {
            it.copy(
                activeChat = null,
                messages = emptyList(),
                draft = "",
                loadingMessages = false,
                sending = false,
                messagesError = null,
                errorMessage = null,
            )
        }
    }

    fun updateDraft(value: String) {
        _state.update { it.copy(draft = value) }
    }

    fun sendMessage() {
        val current = _state.value
        val currentApi = api ?: return
        val sessionId = current.selectedSessionId ?: return
        val chat = current.activeChat ?: return
        val text = current.draft.trim()
        if (text.isEmpty() || current.sending) return
        if (text.codePointCount(0, text.length) > MAX_MESSAGE_CODE_POINTS) {
            _state.update { it.copy(errorMessage = "Сообщение длиннее 4096 символов") }
            return
        }

        _state.update { it.copy(sending = true, errorMessage = null) }
        sendJob = viewModelScope.launch {
            runCatching { currentApi.send(sessionId, chat.peerId, text) }
                .onSuccess { response ->
                    if (api === currentApi &&
                        _state.value.selectedSessionId == sessionId &&
                        _state.value.activeChat?.peerId == chat.peerId
                    ) {
                        chatRevision += 1
                        _state.update {
                            it.copy(
                                draft = if (it.draft.trim() == text) "" else it.draft,
                                sending = false,
                                messages = it.messages.upsert(response.message),
                                chats = it.chats.withPreview(chat.peerId, response.message),
                            )
                        }
                    }
                }
                .onFailure { failure ->
                    if (failure !is CancellationException &&
                        api === currentApi &&
                        _state.value.selectedSessionId == sessionId &&
                        _state.value.activeChat?.peerId == chat.peerId
                    ) {
                        _state.update {
                            it.copy(
                                sending = false,
                                errorMessage = failure.userMessage(),
                            )
                        }
                    }
                }
        }
    }

    fun dismissError() {
        _state.update { it.copy(errorMessage = null) }
    }

    private fun connect(rawBaseUrl: String, preferredSessionId: String?) {
        val candidate = runCatching { apiFactory(rawBaseUrl) }
            .getOrElse { failure ->
                _state.update { it.copy(errorMessage = failure.userMessage()) }
                return
            }
        connectJob?.cancel()
        chatsJob?.cancel()
        messagesJob?.cancel()
        sendJob?.cancel()
        socketJob?.cancel()
        api = candidate
        chatRevision = 0
        connectJob = viewModelScope.launch {
            _state.update {
                it.copy(
                    baseUrlDraft = candidate.normalizedBaseUrl,
                    baseUrl = candidate.normalizedBaseUrl,
                    sessions = emptyList(),
                    selectedSessionId = null,
                    chats = emptyList(),
                    activeChat = null,
                    messages = emptyList(),
                    draft = "",
                    loadingSessions = true,
                    loadingChats = false,
                    loadingMessages = false,
                    sending = false,
                    messagesError = null,
                    connection = ConnectionStatus.Connecting,
                    errorMessage = null,
                )
            }

            runCatching { candidate.sessions() }
                .onSuccess { sessions ->
                    if (api !== candidate) return@onSuccess
                    if (sessions.isEmpty()) {
                        _state.update {
                            it.copy(
                                loadingSessions = false,
                                connection = ConnectionStatus.Offline,
                                errorMessage = "На сервере нет готовых Telegram-аккаунтов",
                            )
                        }
                        return@onSuccess
                    }
                    val selected = sessions.firstOrNull { it.id == preferredSessionId }
                        ?: sessions.firstOrNull(SessionSummary::isDefault)
                        ?: sessions.first()
                    preferences.saveBaseUrl(candidate.normalizedBaseUrl)
                    preferences.saveSessionId(selected.id)
                    _state.update {
                        it.copy(
                            sessions = sessions,
                            selectedSessionId = selected.id,
                            loadingSessions = false,
                            connection = if (foreground) {
                                ConnectionStatus.Connecting
                            } else {
                                ConnectionStatus.Idle
                            },
                        )
                    }
                    refreshChats(candidate, selected.id)
                    observeEvents(candidate, selected.id)
                }
                .onFailure { failure ->
                    if (failure !is CancellationException && api === candidate) {
                        _state.update {
                            it.copy(
                                loadingSessions = false,
                                connection = ConnectionStatus.Offline,
                                errorMessage = failure.userMessage(),
                            )
                        }
                    }
                }
        }
    }

    private fun loadMessages(
        currentApi: RelayClient,
        sessionId: String,
        chat: ChatSummary,
        clearExisting: Boolean,
    ) {
        messagesJob?.cancel()
        _state.update {
            it.copy(
                messages = if (clearExisting) emptyList() else it.messages,
                loadingMessages = true,
                messagesError = null,
            )
        }
        messagesJob = viewModelScope.launch {
            runCatching { currentApi.messages(sessionId, chat.peerId) }
                .onSuccess { loaded ->
                    if (api === currentApi &&
                        _state.value.selectedSessionId == sessionId &&
                        _state.value.activeChat?.peerId == chat.peerId
                    ) {
                        _state.update {
                            it.copy(
                                messages = loaded.mergeMessages(it.messages),
                                loadingMessages = false,
                                messagesError = null,
                            )
                        }
                    }
                }
                .onFailure { failure ->
                    if (failure !is CancellationException &&
                        api === currentApi &&
                        _state.value.selectedSessionId == sessionId &&
                        _state.value.activeChat?.peerId == chat.peerId
                    ) {
                        val message = failure.userMessage()
                        _state.update {
                            it.copy(
                                loadingMessages = false,
                                messagesError = message,
                                errorMessage = message,
                            )
                        }
                    }
                }
        }
    }

    private fun refreshChats(currentApi: RelayClient, sessionId: String) {
        chatsJob?.cancel()
        _state.update { it.copy(loadingChats = true) }
        val requestRevision = chatRevision
        chatsJob = viewModelScope.launch {
            runCatching { currentApi.chats(sessionId) }
                .onSuccess { chats ->
                    if (api === currentApi && _state.value.selectedSessionId == sessionId) {
                        val currentChats = _state.value.chats
                        val sorted = if (chatRevision == requestRevision) {
                            chats.sortedForDisplay()
                        } else {
                            chats.mergeChats(currentChats)
                        }
                        val activePeer = _state.value.activeChat?.peerId
                        _state.update {
                            it.copy(
                                chats = sorted,
                                activeChat = sorted.firstOrNull { chat ->
                                    chat.peerId == activePeer
                                } ?: it.activeChat,
                                loadingChats = false,
                                errorMessage = null,
                            )
                        }
                    }
                }
                .onFailure { failure ->
                    if (failure !is CancellationException &&
                        api === currentApi &&
                        _state.value.selectedSessionId == sessionId
                    ) {
                        _state.update {
                            it.copy(
                                loadingChats = false,
                                errorMessage = failure.userMessage(),
                            )
                        }
                    }
                }
        }
    }

    private fun observeEvents(currentApi: RelayClient, sessionId: String) {
        if (!foreground) return
        socketJob?.cancel()
        socketJob = viewModelScope.launch {
            var attempt = 0
            while (isActive && foreground && _state.value.selectedSessionId == sessionId) {
                var connectedAtNanos: Long? = null
                _state.update {
                    it.copy(
                        connection = if (attempt == 0) {
                            ConnectionStatus.Connecting
                        } else {
                            ConnectionStatus.Offline
                        },
                    )
                }
                try {
                    currentApi.events(sessionId).collect { socketEvent ->
                        when (socketEvent) {
                            SocketEvent.Connected -> {
                                connectedAtNanos = System.nanoTime()
                                _state.update { it.copy(connection = ConnectionStatus.Online) }
                                refreshChats(currentApi, sessionId)
                                _state.value.activeChat?.let { chat ->
                                    loadMessages(
                                        currentApi = currentApi,
                                        sessionId = sessionId,
                                        chat = chat,
                                        clearExisting = false,
                                    )
                                }
                            }

                            is SocketEvent.Payload -> handleEvent(sessionId, socketEvent.event)
                        }
                    }
                } catch (failure: CancellationException) {
                    throw failure
                } catch (_: Throwable) {
                    _state.update { it.copy(connection = ConnectionStatus.Offline) }
                }
                val stableConnection = connectedAtNanos?.let {
                    System.nanoTime() - it >= STABLE_CONNECTION_NANOS
                } ?: false
                attempt = if (stableConnection) 0 else attempt + 1
                delay(reconnectDelay(attempt))
            }
        }
    }

    private fun handleEvent(sessionId: String, event: WsEvent) {
        if (_state.value.selectedSessionId != sessionId) return
        when (event.type) {
            "new_message" -> {
                val peerId = event.peerId ?: return
                val message = event.message ?: return
                val chatKnown = _state.value.chats.any { it.peerId == peerId }
                chatRevision += 1
                _state.update { current ->
                    current.copy(
                        messages = if (current.activeChat?.peerId == peerId) {
                            current.messages.upsert(message)
                        } else {
                            current.messages
                        },
                        chats = current.chats.withPreview(peerId, message),
                    )
                }
                if (!chatKnown) refreshChats()
            }

            "chat_pinned" -> {
                val peerId = event.peerId ?: return
                val pinned = event.pinned ?: return
                chatRevision += 1
                _state.update { current ->
                    val chats = current.chats
                        .map { if (it.peerId == peerId) it.copy(pinned = pinned) else it }
                        .sortedForDisplay()
                    current.copy(
                        chats = chats,
                        activeChat = current.activeChat?.let {
                            if (it.peerId == peerId) it.copy(pinned = pinned) else it
                        },
                    )
                }
            }
        }
    }

    private fun reconnectDelay(attempt: Int): Long =
        (1_000L shl attempt.coerceIn(0, 5)).coerceAtMost(30_000L)

    companion object {
        const val MAX_MESSAGE_CODE_POINTS = 4096

        fun factory(preferences: RelaySettingsStore): ViewModelProvider.Factory = viewModelFactory {
            initializer { LnlViewModel(preferences) }
        }
    }
}

private fun List<Message>.upsert(message: Message): List<Message> =
    (filterNot { it.id == message.id } + message)
        .sortedBy(Message::id)
        .takeLast(MAX_RETAINED_MESSAGES)

private fun List<Message>.mergeMessages(current: List<Message>): List<Message> =
    (this + current)
        .associateBy(Message::id)
        .values
        .sortedBy(Message::id)
        .takeLast(MAX_RETAINED_MESSAGES)

private fun List<ChatSummary>.withPreview(peerId: Long, message: Message): List<ChatSummary> =
    map { chat ->
        if (chat.peerId == peerId) chat.copy(lastMessage = message.preview()) else chat
    }.sortedForDisplay()

private fun List<ChatSummary>.mergeChats(
    current: List<ChatSummary>,
): List<ChatSummary> =
    (this + current)
        .associateBy(ChatSummary::peerId)
        .values
        .toList()
        .sortedForDisplay()

private fun List<ChatSummary>.sortedForDisplay(): List<ChatSummary> =
    sortedWith(
        compareByDescending<ChatSummary>(ChatSummary::pinned)
            .thenBy { it.title.lowercase(Locale.getDefault()) },
    )

private fun Message.preview(): String {
    if (text.isNotBlank()) return text
    return when (media?.kind) {
        "sticker" -> "Стикер"
        "photo" -> "Фото"
        "file" -> "Файл"
        "audio" -> "Аудио"
        "video" -> "Видео"
        "voice" -> "Голосовое сообщение"
        "video_note" -> "Видеосообщение"
        else -> "Вложение"
    }
}

private fun Throwable.userMessage(): String =
    message?.takeIf(String::isNotBlank) ?: "Не удалось выполнить запрос"
