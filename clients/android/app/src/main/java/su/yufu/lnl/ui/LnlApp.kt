package su.yufu.lnl.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.launch
import su.yufu.lnl.BuildConfig
import su.yufu.lnl.data.ChatSummary
import su.yufu.lnl.data.LnlApi
import su.yufu.lnl.data.Message

private sealed interface Screen {
    data object Chats : Screen
    data class Chat(val peerId: Long, val title: String) : Screen
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LnlApp() {
    var baseUrl by remember { mutableStateOf(BuildConfig.DEFAULT_BASE_URL) }
    var screen by remember { mutableStateOf<Screen>(Screen.Chats) }
    var chats by remember { mutableStateOf<List<ChatSummary>>(emptyList()) }
    var messages by remember { mutableStateOf<List<Message>>(emptyList()) }
    var draft by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var online by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val api = remember(baseUrl) { LnlApi(baseUrl) }
    val listState = rememberLazyListState()

    fun refreshChats() {
        scope.launch {
            runCatching { api.chats() }
                .onSuccess {
                    chats = it
                    error = null
                }
                .onFailure { error = it.message }
        }
    }

    LaunchedEffect(api) {
        refreshChats()
        api.events()
            .catch {
                online = false
                error = it.message
            }
            .collect { ev ->
                online = true
                if (ev.type == "new_message" && ev.peerId != null && ev.message != null) {
                    val peer = ev.peerId
                    val msg = ev.message
                    if (screen is Screen.Chat && (screen as Screen.Chat).peerId == peer) {
                        if (messages.none { it.id == msg.id }) {
                            messages = messages + msg
                        }
                    }
                    refreshChats()
                }
            }
    }

    LaunchedEffect(messages.size) {
        if (messages.isNotEmpty()) {
            listState.animateScrollToItem(messages.lastIndex)
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Text(
                        when (val s = screen) {
                            Screen.Chats -> "LNL"
                            is Screen.Chat -> s.title
                        }
                    )
                },
                navigationIcon = {
                    if (screen is Screen.Chat) {
                        TextButton(onClick = { screen = Screen.Chats }) { Text("←") }
                    }
                },
                actions = {
                    Text(
                        if (online) "online" else "…",
                        modifier = Modifier.padding(end = 12.dp),
                        style = MaterialTheme.typography.labelMedium,
                    )
                },
            )
        },
    ) { padding ->
        when (val s = screen) {
            Screen.Chats -> Column(
                Modifier
                    .fillMaxSize()
                    .padding(padding)
                    .padding(12.dp),
            ) {
                OutlinedTextField(
                    value = baseUrl,
                    onValueChange = { baseUrl = it },
                    label = { Text("Relay URL") },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                )
                Row(
                    Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Button(onClick = { refreshChats() }) { Text("Обновить") }
                }
                error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
                LazyColumn(Modifier.fillMaxSize()) {
                    items(chats, key = { it.peerId }) { chat ->
                        Card(
                            Modifier
                                .fillMaxWidth()
                                .padding(vertical = 4.dp)
                                .clickable {
                                    scope.launch {
                                        runCatching { api.messages(chat.peerId) }
                                            .onSuccess {
                                                messages = it
                                                screen = Screen.Chat(chat.peerId, chat.title)
                                                error = null
                                            }
                                            .onFailure { error = it.message }
                                    }
                                },
                        ) {
                            Column(Modifier.padding(12.dp)) {
                                Text(chat.title, style = MaterialTheme.typography.titleMedium)
                                Text(
                                    chat.lastMessage.orEmpty(),
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                    }
                }
            }

            is Screen.Chat -> Column(
                Modifier
                    .fillMaxSize()
                    .padding(padding),
            ) {
                error?.let {
                    Text(
                        it,
                        color = MaterialTheme.colorScheme.error,
                        modifier = Modifier.padding(8.dp),
                    )
                }
                LazyColumn(
                    state = listState,
                    modifier = Modifier
                        .weight(1f)
                        .fillMaxWidth(),
                    contentPadding = PaddingValues(12.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    items(messages, key = { it.id }) { msg ->
                        Box(Modifier.fillMaxWidth()) {
                            Card(
                                modifier = Modifier.align(
                                    if (msg.outgoing) Alignment.CenterEnd else Alignment.CenterStart,
                                ),
                            ) {
                                Text(
                                    msg.text,
                                    modifier = Modifier.padding(10.dp),
                                )
                            }
                        }
                    }
                }
                Row(
                    Modifier
                        .fillMaxWidth()
                        .padding(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    OutlinedTextField(
                        value = draft,
                        onValueChange = { draft = it },
                        modifier = Modifier.weight(1f),
                        placeholder = { Text("Сообщение") },
                    )
                    Button(
                        onClick = {
                            val text = draft.trim()
                            if (text.isEmpty()) return@Button
                            scope.launch {
                                runCatching { api.send(s.peerId, text) }
                                    .onSuccess { res ->
                                        draft = ""
                                        if (messages.none { it.id == res.message.id }) {
                                            messages = messages + res.message
                                        }
                                        refreshChats()
                                    }
                                    .onFailure { error = it.message }
                            }
                        },
                    ) { Text("→") }
                }
            }
        }
    }
}
