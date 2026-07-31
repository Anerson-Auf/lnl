package su.yufu.lnl.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import su.yufu.lnl.R
import su.yufu.lnl.data.ChatSummary
import su.yufu.lnl.data.SessionSummary
import su.yufu.lnl.ui.theme.LocalLnlPalette

@Composable
internal fun ChatsScreen(
    state: LnlUiState,
    padding: PaddingValues,
    onRefresh: () -> Unit,
    onOpenChat: (ChatSummary) -> Unit,
    onSelectSession: (String) -> Unit,
    onOpenRelay: () -> Unit,
) {
    var searchQuery by rememberSaveable(state.selectedSessionId) { mutableStateOf("") }
    var pinnedOnly by rememberSaveable(state.selectedSessionId) { mutableStateOf(false) }
    val visibleChats = remember(state.chats, searchQuery, pinnedOnly) {
        filterChats(state.chats, searchQuery, pinnedOnly)
    }

    val screenModifier = Modifier
        .fillMaxSize()
        .padding(padding)
        .consumeWindowInsets(padding)

    when {
        state.loadingSessions && state.sessions.isEmpty() -> {
            Box(modifier = screenModifier) {
                LoadingState(
                    title = "Подключаем Telegram-аккаунты",
                    subtitle = "Проверяем доступные сессии на relay.",
                )
            }
        }

        state.sessions.isEmpty() -> {
            Box(modifier = screenModifier) {
                EmptyState(
                    title = if (state.baseUrlDraft.isBlank()) {
                        "Настрой подключение"
                    } else {
                        "Аккаунтов пока нет"
                    },
                    subtitle = if (state.baseUrlDraft.isBlank()) {
                        "Укажи адрес relay, чтобы загрузить свои диалоги."
                    } else {
                        "Проверь relay и авторизуй Telegram-сессию в защищённой web-панели."
                    },
                    actionLabel = "Открыть настройки",
                    onAction = onOpenRelay,
                )
            }
        }

        else -> {
            LazyColumn(
                modifier = screenModifier,
                contentPadding = PaddingValues(bottom = 4.dp),
            ) {
                if (state.sessions.size > 1) {
                    item(key = "accounts") {
                        SessionStrip(
                            sessions = state.sessions,
                            selectedSessionId = state.selectedSessionId,
                            onSelect = onSelectSession,
                        )
                    }
                }
                item(key = "search") {
                    ChatSearchField(
                        query = searchQuery,
                        onQueryChange = { searchQuery = it },
                    )
                }
                item(key = "filters") {
                    ChatFilters(
                        chats = state.chats,
                        pinnedOnly = pinnedOnly,
                        onPinnedOnlyChange = { pinnedOnly = it },
                    )
                }
                item(key = "header-divider") {
                    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                }
                when {
                    state.loadingChats && state.chats.isEmpty() -> {
                        items(
                            count = 6,
                            key = { index -> "skeleton-$index" },
                        ) { index ->
                            SkeletonChatRow(index)
                        }
                    }

                    state.chats.isEmpty() -> {
                        item(key = "empty-chats") {
                            InlineEmptyState(
                                title = "Диалогов пока нет",
                                subtitle = "Telegram вернул пустой список для выбранного аккаунта.",
                                actionLabel = "Обновить",
                                onAction = onRefresh,
                            )
                        }
                    }

                    visibleChats.isEmpty() -> {
                        item(key = "empty-filter") {
                            InlineEmptyState(
                                title = if (pinnedOnly && searchQuery.isBlank()) {
                                    "Нет закреплённых чатов"
                                } else {
                                    "Ничего не найдено"
                                },
                                subtitle = if (searchQuery.isBlank()) {
                                    "Вернись ко всем диалогам."
                                } else {
                                    "Попробуй изменить запрос."
                                },
                                actionLabel = "Сбросить фильтр",
                                onAction = {
                                    searchQuery = ""
                                    pinnedOnly = false
                                },
                            )
                        }
                    }

                    else -> {
                        items(visibleChats, key = ChatSummary::peerId) { chat ->
                            ChatRow(
                                chat = chat,
                                onClick = { onOpenChat(chat) },
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun SessionStrip(
    sessions: List<SessionSummary>,
    selectedSessionId: String?,
    onSelect: (String) -> Unit,
) {
    LazyRow(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 88.dp)
            .selectableGroup(),
        contentPadding = PaddingValues(horizontal = 12.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        items(sessions, key = SessionSummary::id) { session ->
            val isSelected = session.id == selectedSessionId
            Column(
                modifier = Modifier
                    .widthIn(min = 66.dp, max = 88.dp)
                    .clip(RoundedCornerShape(14.dp))
                    .selectable(
                        selected = isSelected,
                        role = Role.RadioButton,
                        onClick = { onSelect(session.id) },
                    )
                    .semantics {
                        stateDescription = if (isSelected) {
                            "Выбран"
                        } else {
                            "Не выбран"
                        }
                    }
                    .padding(horizontal = 5.dp, vertical = 4.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                InitialAvatar(
                    label = session.id,
                    size = 50.dp,
                    selected = isSelected,
                )
                Text(
                    text = session.id,
                    style = MaterialTheme.typography.labelMedium,
                    color = if (isSelected) {
                        MaterialTheme.colorScheme.primary
                    } else {
                        MaterialTheme.colorScheme.onSurface
                    },
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

@Composable
private fun ChatSearchField(
    query: String,
    onQueryChange: (String) -> Unit,
) {
    TextField(
        value = query,
        onValueChange = onQueryChange,
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 4.dp),
        placeholder = { Text("Поиск по чатам") },
        leadingIcon = {
            Icon(
                imageVector = Icons.Rounded.Search,
                contentDescription = null,
            )
        },
        trailingIcon = if (query.isNotEmpty()) {
            {
                IconButton(onClick = { onQueryChange("") }) {
                    Icon(
                        imageVector = Icons.Rounded.Close,
                        contentDescription = "Очистить поиск",
                    )
                }
            }
        } else {
            null
        },
        shape = RoundedCornerShape(28.dp),
        singleLine = true,
        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
        colors = TextFieldDefaults.colors(
            focusedContainerColor = MaterialTheme.colorScheme.surfaceVariant,
            unfocusedContainerColor = MaterialTheme.colorScheme.surfaceVariant,
            focusedIndicatorColor = Color.Transparent,
            unfocusedIndicatorColor = Color.Transparent,
            disabledIndicatorColor = Color.Transparent,
        ),
    )
}

@Composable
private fun ChatFilters(
    chats: List<ChatSummary>,
    pinnedOnly: Boolean,
    onPinnedOnlyChange: (Boolean) -> Unit,
) {
    val pinnedCount = chats.count(ChatSummary::pinned)
    LazyRow(
        modifier = Modifier
            .fillMaxWidth()
            .selectableGroup(),
        contentPadding = PaddingValues(horizontal = 16.dp, vertical = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        item {
            FilterChip(
                selected = !pinnedOnly,
                onClick = { onPinnedOnlyChange(false) },
                label = { Text("Все · ${chats.size}") },
            )
        }
        item {
            FilterChip(
                selected = pinnedOnly,
                onClick = { onPinnedOnlyChange(true) },
                label = { Text("Закреплённые · $pinnedCount") },
            )
        }
    }
}

@Composable
private fun ChatRow(
    chat: ChatSummary,
    onClick: () -> Unit,
) {
    Surface(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(
                role = Role.Button,
                onClickLabel = "Открыть чат ${chat.title}",
                onClick = onClick,
            ),
        color = MaterialTheme.colorScheme.surface,
    ) {
        Column {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(min = 78.dp)
                    .padding(start = 16.dp, end = 14.dp, top = 10.dp, bottom = 10.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                InitialAvatar(
                    label = chat.title,
                    size = 54.dp,
                )
                Column(
                    modifier = Modifier.weight(1f),
                    verticalArrangement = Arrangement.spacedBy(3.dp),
                ) {
                    Text(
                        text = chat.title,
                        style = MaterialTheme.typography.titleMedium,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(
                        text = chat.lastMessage?.takeIf(String::isNotBlank) ?: "Нет сообщений",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                if (chat.pinned) {
                    Icon(
                        painter = painterResource(R.drawable.ic_push_pin),
                        contentDescription = "Чат закреплён",
                        modifier = Modifier.size(18.dp),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            HorizontalDivider(
                modifier = Modifier.padding(start = 82.dp),
                color = MaterialTheme.colorScheme.outlineVariant,
            )
        }
    }
}

@Composable
internal fun AccountsScreen(
    state: LnlUiState,
    padding: PaddingValues,
    onSelectSession: (String) -> Unit,
    onOpenRelay: () -> Unit,
) {
    if (state.sessions.isEmpty()) {
        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .consumeWindowInsets(padding),
        ) {
            EmptyState(
                title = "Нет доступных аккаунтов",
                subtitle = "Подключи relay или авторизуй Telegram-сессию в web-панели.",
                actionLabel = "Настроить Relay",
                onAction = onOpenRelay,
            )
        }
        return
    }

    LazyColumn(
        modifier = Modifier
            .fillMaxSize()
            .padding(padding)
            .consumeWindowInsets(padding)
            .selectableGroup(),
        contentPadding = PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        item {
            InfoCard(
                icon = Icons.Rounded.Info,
                title = "Выбранный аккаунт запоминается",
                text = "При следующем запуске LNL откроет эту же Telegram-сессию. Переключаться можно без ограничения по количеству аккаунтов.",
            )
        }
        items(state.sessions, key = SessionSummary::id) { session ->
            SessionCard(
                session = session,
                selected = session.id == state.selectedSessionId,
                connection = state.connection,
                onClick = { onSelectSession(session.id) },
            )
        }
        item {
            TextButton(
                onClick = onOpenRelay,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text("Настройки Relay")
            }
        }
    }
}

@Composable
private fun SessionCard(
    session: SessionSummary,
    selected: Boolean,
    connection: ConnectionStatus,
    onClick: () -> Unit,
) {
    val palette = LocalLnlPalette.current
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .selectable(
                selected = selected,
                role = Role.RadioButton,
                onClick = onClick,
            )
            .semantics {
                stateDescription = if (selected) "Текущий аккаунт" else "Доступный аккаунт"
            },
        shape = RoundedCornerShape(18.dp),
        colors = CardDefaults.cardColors(
            containerColor = if (selected) {
                MaterialTheme.colorScheme.primaryContainer
            } else {
                MaterialTheme.colorScheme.surface
            },
        ),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(min = 78.dp)
                .padding(14.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            InitialAvatar(
                label = session.id,
                size = 52.dp,
                selected = selected,
            )
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(3.dp),
            ) {
                Row(
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = session.id,
                        style = MaterialTheme.typography.titleMedium,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    if (session.isDefault) {
                        Surface(
                            shape = RoundedCornerShape(8.dp),
                            color = MaterialTheme.colorScheme.secondaryContainer,
                        ) {
                            Text(
                                text = "основной",
                                modifier = Modifier.padding(horizontal = 7.dp, vertical = 2.dp),
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSecondaryContainer,
                            )
                        }
                    }
                }
                Text(
                    text = if (selected) {
                        "Текущий · ${connection.label()}"
                    } else {
                        "Готовая Telegram-сессия"
                    },
                    style = MaterialTheme.typography.bodyMedium,
                    color = if (selected && connection == ConnectionStatus.Online) {
                        palette.online
                    } else {
                        MaterialTheme.colorScheme.onSurfaceVariant
                    },
                )
            }
            if (selected) {
                Icon(
                    imageVector = Icons.Rounded.CheckCircle,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.primary,
                )
            }
        }
    }
}

@Composable
internal fun RelayScreen(
    state: LnlUiState,
    padding: PaddingValues,
    allowCleartext: Boolean,
    onBaseUrlChange: (String) -> Unit,
    onConnect: () -> Unit,
) {
    LazyColumn(
        modifier = Modifier
            .fillMaxSize()
            .padding(padding)
            .consumeWindowInsets(padding),
        contentPadding = PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        item {
            ConnectionCard(state)
        }
        item {
            Card(
                colors = CardDefaults.cardColors(
                    containerColor = MaterialTheme.colorScheme.surface,
                ),
                shape = RoundedCornerShape(20.dp),
            ) {
                Column(
                    modifier = Modifier.padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Text(
                        text = "Адрес relay",
                        style = MaterialTheme.typography.titleMedium,
                    )
                    Text(
                        text = "Здесь LNL получает список аккаунтов, диалоги и live-события.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    OutlinedTextField(
                        value = state.baseUrlDraft,
                        onValueChange = onBaseUrlChange,
                        label = { Text("Relay URL") },
                        placeholder = { Text("https://relay.example.com") },
                        modifier = Modifier.fillMaxWidth(),
                        singleLine = true,
                        keyboardOptions = KeyboardOptions(
                            keyboardType = KeyboardType.Uri,
                            imeAction = ImeAction.Done,
                        ),
                        keyboardActions = KeyboardActions(
                            onDone = {
                                if (state.baseUrlDraft.isNotBlank() &&
                                    !state.loadingSessions
                                ) {
                                    onConnect()
                                }
                            },
                        ),
                        shape = RoundedCornerShape(14.dp),
                        colors = OutlinedTextFieldDefaults.colors(
                            focusedContainerColor = MaterialTheme.colorScheme.surface,
                            unfocusedContainerColor = MaterialTheme.colorScheme.surface,
                        ),
                    )
                    Button(
                        onClick = onConnect,
                        enabled = state.baseUrlDraft.isNotBlank() && !state.loadingSessions,
                        modifier = Modifier
                            .fillMaxWidth()
                            .heightIn(min = 50.dp),
                    ) {
                        if (state.loadingSessions) {
                            CircularProgressIndicator(
                                modifier = Modifier.size(20.dp),
                                strokeWidth = 2.dp,
                                color = MaterialTheme.colorScheme.onPrimary,
                            )
                            Spacer(Modifier.width(10.dp))
                            Text("Подключаемся…")
                        } else {
                            Text("Подключиться")
                        }
                    }
                }
            }
        }
        item {
            InfoCard(
                icon = Icons.Rounded.Settings,
                title = "Безопасность соединения",
                text = if (allowCleartext) {
                    "Debug-сборка разрешает HTTP для локальной разработки. Для внешнего relay используй HTTPS и сетевое ограничение доступа."
                } else {
                    "Release-сборка принимает только HTTPS. Публичный API пока без client-auth, поэтому relay нельзя открывать в интернет без VPN или другого сетевого ограничения."
                },
            )
        }
        item {
            Text(
                text = "Relay URL и id выбранной сессии сохраняются локально. Токены и пароли приложение не хранит.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 12.dp),
            )
        }
    }
}

@Composable
private fun ConnectionCard(state: LnlUiState) {
    val palette = LocalLnlPalette.current
    val statusColor = when (state.connection) {
        ConnectionStatus.Idle -> MaterialTheme.colorScheme.onSurfaceVariant
        ConnectionStatus.Connecting -> palette.warning
        ConnectionStatus.Online -> palette.online
        ConnectionStatus.Offline -> MaterialTheme.colorScheme.error
    }
    Card(
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.primaryContainer,
        ),
        shape = RoundedCornerShape(20.dp),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            InitialAvatar(
                label = state.selectedSessionId ?: "LNL",
                size = 50.dp,
            )
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(3.dp),
            ) {
                Text(
                    text = state.selectedSessionId ?: "Аккаунт не выбран",
                    style = MaterialTheme.typography.titleMedium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = state.connection.label(),
                    style = MaterialTheme.typography.bodyMedium,
                    color = statusColor,
                )
                if (state.baseUrl.isNotBlank()) {
                    Text(
                        text = "Активный: ${state.baseUrl}",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onPrimaryContainer,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                if (state.baseUrlDraft != state.baseUrl) {
                    Text(
                        text = "Новый URL ещё не применён",
                        style = MaterialTheme.typography.labelSmall,
                        color = palette.warning,
                    )
                }
            }
            Text(
                text = "${state.sessions.size} акк.",
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onPrimaryContainer,
            )
        }
    }
}

@Composable
private fun InfoCard(
    icon: ImageVector,
    title: String,
    text: String,
) {
    Card(
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant,
        ),
        shape = RoundedCornerShape(18.dp),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.Top,
        ) {
            Surface(
                modifier = Modifier.size(38.dp),
                shape = CircleShape,
                color = MaterialTheme.colorScheme.primaryContainer,
            ) {
                Box(contentAlignment = Alignment.Center) {
                    Icon(
                        imageVector = icon,
                        contentDescription = null,
                        modifier = Modifier.size(20.dp),
                        tint = MaterialTheme.colorScheme.primary,
                    )
                }
            }
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(3.dp),
            ) {
                Text(
                    text = title,
                    style = MaterialTheme.typography.titleSmall,
                )
                Text(
                    text = text,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
private fun LoadingState(
    title: String,
    subtitle: String,
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically),
    ) {
        CircularProgressIndicator()
        Text(
            text = title,
            style = MaterialTheme.typography.titleMedium,
            textAlign = TextAlign.Center,
        )
        Text(
            text = subtitle,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
    }
}

@Composable
internal fun EmptyState(
    title: String,
    subtitle: String,
    modifier: Modifier = Modifier,
    actionLabel: String? = null,
    onAction: (() -> Unit)? = null,
) {
    Column(
        modifier = modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterVertically),
    ) {
        Surface(
            modifier = Modifier.size(72.dp),
            shape = CircleShape,
            color = MaterialTheme.colorScheme.primaryContainer,
        ) {
            Box(contentAlignment = Alignment.Center) {
                Text(
                    text = "LNL",
                    color = MaterialTheme.colorScheme.primary,
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Bold,
                )
            }
        }
        Spacer(Modifier.height(4.dp))
        Text(
            text = title,
            style = MaterialTheme.typography.titleMedium,
            textAlign = TextAlign.Center,
        )
        Text(
            text = subtitle,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
        if (actionLabel != null && onAction != null) {
            Button(onClick = onAction) {
                Text(actionLabel)
            }
        }
    }
}

@Composable
private fun InlineEmptyState(
    title: String,
    subtitle: String,
    actionLabel: String,
    onAction: () -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 24.dp, vertical = 40.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(
            text = title,
            style = MaterialTheme.typography.titleMedium,
            textAlign = TextAlign.Center,
        )
        Text(
            text = subtitle,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
        Button(onClick = onAction) {
            Text(actionLabel)
        }
    }
}

@Composable
private fun SkeletonChatRow(index: Int) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 78.dp)
            .padding(horizontal = 16.dp, vertical = 10.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Spacer(
            modifier = Modifier
                .size(54.dp)
                .background(
                    color = MaterialTheme.colorScheme.surfaceVariant,
                    shape = CircleShape,
                ),
        )
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Spacer(
                modifier = Modifier
                    .fillMaxWidth(if (index % 2 == 0) 0.48f else 0.62f)
                    .height(14.dp)
                    .background(
                        color = MaterialTheme.colorScheme.surfaceVariant,
                        shape = RoundedCornerShape(8.dp),
                    ),
            )
            Spacer(
                modifier = Modifier
                    .fillMaxWidth(if (index % 3 == 0) 0.82f else 0.68f)
                    .height(12.dp)
                    .background(
                        color = MaterialTheme.colorScheme.surfaceVariant,
                        shape = RoundedCornerShape(8.dp),
                    ),
            )
        }
    }
    HorizontalDivider(
        modifier = Modifier.padding(start = 82.dp),
        color = MaterialTheme.colorScheme.outlineVariant,
    )
}
