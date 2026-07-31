package su.yufu.lnl.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import java.text.DateFormat
import java.util.Date
import su.yufu.lnl.BuildConfig
import su.yufu.lnl.data.ChatSummary
import su.yufu.lnl.data.MediaInfo
import su.yufu.lnl.data.Message
import su.yufu.lnl.data.RelayPreferences
import su.yufu.lnl.data.SessionSummary

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LnlApp() {
    val context = LocalContext.current.applicationContext
    val preferences = remember {
        RelayPreferences(context, BuildConfig.DEFAULT_BASE_URL)
    }
    val viewModel: LnlViewModel = viewModel(factory = LnlViewModel.factory(preferences))
    val state by viewModel.state.collectAsStateWithLifecycle()
    val snackbarHostState = remember { SnackbarHostState() }
    val lifecycleOwner = LocalLifecycleOwner.current

    DisposableEffect(lifecycleOwner, viewModel) {
        val lifecycle = lifecycleOwner.lifecycle
        val observer = LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_START -> viewModel.setForeground(true)
                Lifecycle.Event.ON_STOP -> viewModel.setForeground(false)
                else -> Unit
            }
        }
        lifecycle.addObserver(observer)
        viewModel.setForeground(
            lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED),
        )
        onDispose {
            lifecycle.removeObserver(observer)
            viewModel.setForeground(false)
        }
    }

    LaunchedEffect(state.errorMessage) {
        val message = state.errorMessage ?: return@LaunchedEffect
        snackbarHostState.showSnackbar(message)
        viewModel.dismissError()
    }

    BackHandler(enabled = state.activeChat != null) {
        viewModel.closeChat()
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        Text(
                            text = state.activeChat?.title ?: "LNL",
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        state.selectedSessionId?.let {
                            Text(
                                text = "Аккаунт: $it",
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                },
                navigationIcon = {
                    if (state.activeChat != null) {
                        TextButton(onClick = viewModel::closeChat) {
                            Text("Назад")
                        }
                    }
                },
                actions = {
                    ConnectionBadge(state.connection)
                    SessionMenu(
                        sessions = state.sessions,
                        selectedSessionId = state.selectedSessionId,
                        onSelect = viewModel::selectSession,
                    )
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbarHostState) },
    ) { padding ->
        if (state.activeChat == null) {
            ChatsScreen(
                state = state,
                padding = padding,
                onBaseUrlChange = viewModel::updateBaseUrlDraft,
                onConnect = viewModel::connect,
                onRefresh = viewModel::refreshChats,
                onOpenChat = viewModel::openChat,
            )
        } else {
            ChatScreen(
                state = state,
                padding = padding,
                onDraftChange = viewModel::updateDraft,
                onSend = viewModel::sendMessage,
                onRetry = viewModel::retryMessages,
            )
        }
    }
}

@Composable
private fun ChatsScreen(
    state: LnlUiState,
    padding: PaddingValues,
    onBaseUrlChange: (String) -> Unit,
    onConnect: () -> Unit,
    onRefresh: () -> Unit,
    onOpenChat: (ChatSummary) -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(padding),
    ) {
        Column(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 10.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            OutlinedTextField(
                value = state.baseUrlDraft,
                onValueChange = onBaseUrlChange,
                label = { Text("Relay URL") },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true,
                keyboardOptions = KeyboardOptions(
                    keyboardType = KeyboardType.Uri,
                    imeAction = ImeAction.Done,
                ),
                keyboardActions = KeyboardActions(onDone = { onConnect() }),
            )
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Button(
                    onClick = onConnect,
                    enabled = !state.loadingSessions,
                ) {
                    Text(if (state.loadingSessions) "Подключение…" else "Подключиться")
                }
                TextButton(
                    onClick = onRefresh,
                    enabled = state.selectedSessionId != null && !state.loadingChats,
                ) {
                    Text("Обновить диалоги")
                }
            }
            if (state.loadingSessions || state.loadingChats) {
                LinearProgressIndicator(Modifier.fillMaxWidth())
            }
        }

        HorizontalDivider()
        when {
            state.loadingSessions && state.sessions.isEmpty() -> {
                CenterMessage("Ищем готовые Telegram-аккаунты…")
            }

            state.sessions.isEmpty() -> {
                CenterMessage(
                    title = if (state.baseUrlDraft.isBlank()) {
                        "Укажи Relay URL"
                    } else {
                        "Нет доступных аккаунтов"
                    },
                    subtitle = if (state.baseUrlDraft.isBlank()) {
                        "Release-сборка подключается только по HTTPS."
                    } else {
                        "Проверь Relay URL и авторизуй аккаунт в защищённой web-панели."
                    },
                )
            }

            state.chats.isEmpty() && !state.loadingChats -> {
                CenterMessage(
                    title = "Диалогов пока нет",
                    subtitle = "Telegram вернул пустой список для выбранного аккаунта.",
                )
            }

            else -> {
                LazyColumn(
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(vertical = 6.dp),
                ) {
                    items(state.chats, key = ChatSummary::peerId) { chat ->
                        ListItem(
                            headlineContent = {
                                Text(
                                    text = chat.title,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                )
                            },
                            supportingContent = {
                                Text(
                                    text = chat.lastMessage ?: "Нет сообщений",
                                    maxLines = 2,
                                    overflow = TextOverflow.Ellipsis,
                                )
                            },
                            trailingContent = {
                                if (chat.pinned) {
                                    Text(
                                        text = "Закреплён",
                                        style = MaterialTheme.typography.labelSmall,
                                        color = MaterialTheme.colorScheme.primary,
                                    )
                                }
                            },
                            modifier = Modifier
                                .fillMaxWidth()
                                .clickable { onOpenChat(chat) },
                        )
                        HorizontalDivider(Modifier.padding(horizontal = 16.dp))
                    }
                }
            }
        }
    }
}

@Composable
private fun ChatScreen(
    state: LnlUiState,
    padding: PaddingValues,
    onDraftChange: (String) -> Unit,
    onSend: () -> Unit,
    onRetry: () -> Unit,
) {
    val listState = rememberLazyListState()
    var previousMessageCount by remember(state.activeChat?.peerId) {
        mutableIntStateOf(0)
    }

    LaunchedEffect(
        state.messages.size,
        state.messages.lastOrNull()?.id,
        state.activeChat?.peerId,
    ) {
        if (state.messages.isNotEmpty()) {
            val visibleItems = listState.layoutInfo.visibleItemsInfo
            val wasNearBottom = previousMessageCount == 0 ||
                visibleItems.lastOrNull()?.index?.let { it >= previousMessageCount - 2 } == true
            if (wasNearBottom) {
                if (previousMessageCount == 0) {
                    listState.scrollToItem(state.messages.lastIndex)
                } else {
                    listState.animateScrollToItem(state.messages.lastIndex)
                }
            }
        }
        previousMessageCount = state.messages.size
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(padding)
            .navigationBarsPadding()
            .imePadding(),
    ) {
        if (state.loadingMessages) {
            LinearProgressIndicator(Modifier.fillMaxWidth())
        }
        if (state.activeChat?.pinned == true) {
            Text(
                text = "Закреплён в Telegram",
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.primary,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 6.dp),
            )
        }
        if (state.messagesError != null && state.messages.isNotEmpty()) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 4.dp),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = "Не удалось обновить историю",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                )
                TextButton(onClick = onRetry) {
                    Text("Повторить")
                }
            }
        }
        if (state.messagesError != null && state.messages.isEmpty() && !state.loadingMessages) {
            CenterMessage(
                title = "История не загрузилась",
                subtitle = state.messagesError,
                modifier = Modifier.weight(1f),
                actionLabel = "Повторить",
                onAction = onRetry,
            )
        } else if (state.messages.isEmpty() && !state.loadingMessages) {
            CenterMessage(
                title = "Сообщений пока нет",
                subtitle = "Напиши первое сообщение.",
                modifier = Modifier.weight(1f),
            )
        } else {
            LazyColumn(
                state = listState,
                modifier = Modifier
                    .weight(1f)
                    .fillMaxWidth(),
                contentPadding = PaddingValues(12.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                items(state.messages, key = Message::id) { message ->
                    MessageBubble(message)
                }
            }
        }

        HorizontalDivider()
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(8.dp),
            verticalAlignment = Alignment.Bottom,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            OutlinedTextField(
                value = state.draft,
                onValueChange = onDraftChange,
                modifier = Modifier.weight(1f),
                placeholder = { Text("Сообщение") },
                minLines = 1,
                maxLines = 5,
                enabled = !state.sending,
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send),
                keyboardActions = KeyboardActions(onSend = { onSend() }),
            )
            Button(
                onClick = onSend,
                enabled = state.draft.isNotBlank() && !state.sending,
                modifier = Modifier.sizeIn(minHeight = 56.dp),
            ) {
                if (state.sending) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(20.dp),
                        strokeWidth = 2.dp,
                    )
                } else {
                    Text("Отправить")
                }
            }
        }
    }
}

@Composable
private fun MessageBubble(message: Message) {
    val alignment = if (message.outgoing) Alignment.CenterEnd else Alignment.CenterStart
    val color = if (message.outgoing) {
        MaterialTheme.colorScheme.primaryContainer
    } else {
        MaterialTheme.colorScheme.surfaceVariant
    }

    Box(Modifier.fillMaxWidth()) {
        Surface(
            modifier = Modifier
                .align(alignment)
                .sizeIn(maxWidth = 320.dp),
            color = color,
            shape = RoundedCornerShape(18.dp),
        ) {
            Column(
                modifier = Modifier.padding(horizontal = 12.dp, vertical = 9.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                message.media?.let { MediaSummary(it) }
                if (message.text.isNotBlank()) {
                    SelectionContainer {
                        Text(
                            text = message.text,
                            style = MaterialTheme.typography.bodyLarge,
                        )
                    }
                }
                Text(
                    text = message.timeLabel(),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.align(Alignment.End),
                )
            }
        }
    }
}

@Composable
private fun MediaSummary(media: MediaInfo) {
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Text(
            text = media.title(),
            style = MaterialTheme.typography.titleSmall,
            fontWeight = FontWeight.SemiBold,
        )
        val details = listOfNotNull(
            media.fileName,
            media.size?.let(::formatBytes),
            media.durationSeconds?.let(::formatDuration),
            if (media.spoiler) "скрытое медиа" else null,
        ).joinToString(" · ")
        if (details.isNotBlank()) {
            Text(
                text = details,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@Composable
private fun SessionMenu(
    sessions: List<SessionSummary>,
    selectedSessionId: String?,
    onSelect: (String) -> Unit,
) {
    if (sessions.isEmpty()) return
    var expanded by remember { mutableStateOf(false) }
    Box {
        TextButton(onClick = { expanded = true }) {
            Text(selectedSessionId ?: "Аккаунт")
        }
        DropdownMenu(
            expanded = expanded,
            onDismissRequest = { expanded = false },
        ) {
            sessions.forEach { session ->
                DropdownMenuItem(
                    text = {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text(session.id)
                            if (session.isDefault) {
                                Spacer(Modifier.width(8.dp))
                                Text(
                                    text = "основной",
                                    style = MaterialTheme.typography.labelSmall,
                                    color = MaterialTheme.colorScheme.primary,
                                )
                            }
                        }
                    },
                    onClick = {
                        expanded = false
                        onSelect(session.id)
                    },
                )
            }
        }
    }
}

@Composable
private fun ConnectionBadge(status: ConnectionStatus) {
    val (label, color) = when (status) {
        ConnectionStatus.Idle -> "не подключён" to MaterialTheme.colorScheme.onSurfaceVariant
        ConnectionStatus.Connecting -> "подключение" to MaterialTheme.colorScheme.tertiary
        ConnectionStatus.Online -> "онлайн" to MaterialTheme.colorScheme.primary
        ConnectionStatus.Offline -> "офлайн" to MaterialTheme.colorScheme.error
    }
    Text(
        text = label,
        style = MaterialTheme.typography.labelSmall,
        color = color,
    )
}

@Composable
private fun CenterMessage(
    title: String,
    modifier: Modifier = Modifier,
    subtitle: String? = null,
    actionLabel: String? = null,
    onAction: (() -> Unit)? = null,
) {
    Box(
        modifier = modifier
            .fillMaxSize()
            .padding(24.dp),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Text(title, style = MaterialTheme.typography.titleMedium)
            subtitle?.let {
                Text(
                    text = it,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            if (actionLabel != null && onAction != null) {
                TextButton(onClick = onAction) {
                    Text(actionLabel)
                }
            }
        }
    }
}

private fun MediaInfo.title(): String = when (kind) {
    "sticker" -> emoji?.let { "Стикер $it" } ?: "Стикер"
    "photo" -> "Фотография"
    "file" -> "Файл"
    "audio" -> "Аудио"
    "video" -> "Видео"
    "voice" -> "Голосовое сообщение"
    "video_note" -> "Видеосообщение"
    else -> "Вложение"
}

private fun Message.timeLabel(): String =
    DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(date.toLong() * 1_000))

private fun formatDuration(seconds: Int): String =
    "%d:%02d".format(seconds / 60, seconds % 60)

private fun formatBytes(bytes: Long): String = when {
    bytes < 1_024 -> "$bytes Б"
    bytes < 1_024 * 1_024 -> "%.1f КиБ".format(bytes / 1_024.0)
    else -> "%.1f МиБ".format(bytes / (1_024.0 * 1_024.0))
}
