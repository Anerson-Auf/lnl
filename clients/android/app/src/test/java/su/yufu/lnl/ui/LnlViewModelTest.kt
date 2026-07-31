package su.yufu.lnl.ui

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import su.yufu.lnl.data.ChatSummary
import su.yufu.lnl.data.Message
import su.yufu.lnl.data.RelayClient
import su.yufu.lnl.data.RelaySettings
import su.yufu.lnl.data.RelaySettingsStore
import su.yufu.lnl.data.SendResponse
import su.yufu.lnl.data.SessionSummary
import su.yufu.lnl.data.SocketEvent
import su.yufu.lnl.data.WsEvent

@OptIn(ExperimentalCoroutinesApi::class)
class LnlViewModelTest {
    private lateinit var dispatcher: TestDispatcher

    @Before
    fun setUp() {
        dispatcher = StandardTestDispatcher()
        Dispatchers.setMain(dispatcher)
    }

    @After
    fun tearDown() {
        Dispatchers.resetMain()
    }

    @Test
    fun restoresSavedSessionFromMoreThanTwoAccounts() = runTest(dispatcher) {
        val client = FakeRelayClient(
            sessionsResult = listOf(
                SessionSummary("main", isDefault = true),
                SessionSummary("work", isDefault = false),
                SessionSummary("family", isDefault = false),
            ),
        )
        val store = FakeSettingsStore(sessionId = "family")
        val viewModel = viewModel(store, client)

        advanceUntilIdle()

        assertEquals("family", viewModel.state.value.selectedSessionId)
        assertEquals("family", store.sessionId)
        assertEquals(listOf("family"), client.chatRequests)
    }

    @Test
    fun historyResponseKeepsMessageReceivedWhileLoading() = runTest(dispatcher) {
        val chat = ChatSummary(peerId = 42, title = "Роман")
        val history = CompletableDeferred<List<Message>>()
        val client = FakeRelayClient(chatsResult = listOf(chat)).apply {
            messagesHandler = { _, _ -> history.await() }
        }
        val viewModel = viewModel(FakeSettingsStore(), client)
        viewModel.setForeground(true)
        advanceUntilIdle()
        viewModel.openChat(chat)
        runCurrent()

        client.events.emit(
            SocketEvent.Payload(
                WsEvent(
                    type = "new_message",
                    peerId = chat.peerId,
                    message = message(id = 2, text = "live"),
                ),
            ),
        )
        runCurrent()
        history.complete(listOf(message(id = 1, text = "history")))
        advanceUntilIdle()

        assertEquals(listOf(1, 2), viewModel.state.value.messages.map(Message::id))
        viewModel.setForeground(false)
    }

    @Test
    fun pinEventUpdatesAndResortsChats() = runTest(dispatcher) {
        val first = ChatSummary(peerId = 1, title = "Альфа")
        val second = ChatSummary(peerId = 2, title = "Бета")
        val client = FakeRelayClient(chatsResult = listOf(first, second))
        val viewModel = viewModel(FakeSettingsStore(), client)
        viewModel.setForeground(true)
        advanceUntilIdle()

        client.events.emit(
            SocketEvent.Payload(
                WsEvent(type = "chat_pinned", peerId = second.peerId, pinned = true),
            ),
        )
        runCurrent()

        assertEquals(second.peerId, viewModel.state.value.chats.first().peerId)
        assertTrue(viewModel.state.value.chats.first().pinned)
        viewModel.setForeground(false)
    }

    @Test
    fun reconnectReconcilesEventsMissedWhileBackgrounded() = runTest(dispatcher) {
        val chat = ChatSummary(peerId = 42, title = "Роман")
        var history = listOf(message(id = 1, text = "до паузы"))
        val client = FakeRelayClient(chatsResult = listOf(chat)).apply {
            messagesHandler = { _, _ -> history }
        }
        val viewModel = viewModel(FakeSettingsStore(), client)
        viewModel.setForeground(true)
        advanceUntilIdle()
        viewModel.openChat(chat)
        advanceUntilIdle()
        viewModel.setForeground(false)

        history = history + message(id = 2, text = "в фоне")
        client.chatsResult = listOf(chat.copy(pinned = true))
        viewModel.setForeground(true)
        runCurrent()
        client.events.emit(SocketEvent.Connected)
        advanceUntilIdle()

        assertEquals(listOf(1, 2), viewModel.state.value.messages.map(Message::id))
        assertTrue(viewModel.state.value.chats.single().pinned)
        assertTrue(viewModel.state.value.activeChat?.pinned == true)
        viewModel.setForeground(false)
    }

    @Test
    fun cancelledSendCannotClearDraftAfterChatIsReopened() = runTest(dispatcher) {
        val chat = ChatSummary(peerId = 42, title = "Роман")
        val pendingSend = CompletableDeferred<SendResponse>()
        val client = FakeRelayClient(chatsResult = listOf(chat)).apply {
            sendHandler = { _, _, _ -> pendingSend.await() }
        }
        val viewModel = viewModel(FakeSettingsStore(), client)
        advanceUntilIdle()
        viewModel.openChat(chat)
        advanceUntilIdle()
        viewModel.updateDraft("старый текст")
        viewModel.sendMessage()
        runCurrent()

        viewModel.closeChat()
        viewModel.openChat(chat)
        viewModel.updateDraft("новый текст")
        pendingSend.complete(SendResponse(ok = true, message = message(9, "sent")))
        advanceUntilIdle()

        assertEquals("новый текст", viewModel.state.value.draft)
        assertFalse(viewModel.state.value.messages.any { it.id == 9 })
    }

    @Test
    fun messageLimitCountsUnicodeCodePoints() = runTest(dispatcher) {
        val chat = ChatSummary(peerId = 42, title = "Роман")
        val client = FakeRelayClient(chatsResult = listOf(chat))
        val viewModel = viewModel(FakeSettingsStore(), client)
        advanceUntilIdle()
        viewModel.openChat(chat)
        advanceUntilIdle()
        viewModel.updateDraft("😀".repeat(LnlViewModel.MAX_MESSAGE_CODE_POINTS + 1))

        viewModel.sendMessage()

        assertEquals(0, client.sendRequests)
        assertEquals(
            "Сообщение длиннее 4096 символов",
            viewModel.state.value.errorMessage,
        )
    }

    private fun viewModel(
        store: FakeSettingsStore,
        client: FakeRelayClient,
    ): LnlViewModel = LnlViewModel(
        preferences = store,
        apiFactory = { client },
    )

    private fun message(id: Int, text: String): Message = Message(
        id = id,
        text = text,
        outgoing = false,
        date = id,
    )
}

private class FakeSettingsStore(
    baseUrl: String = "https://relay.example.test",
    sessionId: String? = null,
) : RelaySettingsStore {
    private var baseUrl = baseUrl
    var sessionId = sessionId
        private set

    override fun load(): RelaySettings = RelaySettings(baseUrl, sessionId)

    override fun saveBaseUrl(baseUrl: String) {
        this.baseUrl = baseUrl
    }

    override fun saveSessionId(sessionId: String) {
        this.sessionId = sessionId
    }
}

private class FakeRelayClient(
    override val normalizedBaseUrl: String = "https://relay.example.test",
    var sessionsResult: List<SessionSummary> = listOf(
        SessionSummary("main", isDefault = true),
    ),
    var chatsResult: List<ChatSummary> = emptyList(),
) : RelayClient {
    val events = MutableSharedFlow<SocketEvent>(extraBufferCapacity = 8)
    val chatRequests = mutableListOf<String>()
    var sendRequests = 0
    var messagesHandler: suspend (String, Long) -> List<Message> = { _, _ -> emptyList() }
    var sendHandler: suspend (String, Long, String) -> SendResponse = { _, _, text ->
        SendResponse(ok = true, message = Message(1, text, true, 1))
    }

    override suspend fun sessions(): List<SessionSummary> = sessionsResult

    override suspend fun chats(sessionId: String): List<ChatSummary> {
        chatRequests += sessionId
        return chatsResult
    }

    override suspend fun messages(sessionId: String, peerId: Long): List<Message> =
        messagesHandler(sessionId, peerId)

    override suspend fun send(
        sessionId: String,
        peerId: Long,
        text: String,
    ): SendResponse {
        sendRequests += 1
        return sendHandler(sessionId, peerId, text)
    }

    override fun events(sessionId: String): Flow<SocketEvent> = events
}
