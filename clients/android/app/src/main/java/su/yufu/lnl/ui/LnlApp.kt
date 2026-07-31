package su.yufu.lnl.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.automirrored.rounded.List
import androidx.compose.material.icons.rounded.Person
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import su.yufu.lnl.BuildConfig
import su.yufu.lnl.data.RelayPreferences
import su.yufu.lnl.ui.theme.LocalLnlPalette

internal enum class HomeDestination(
    val label: String,
) {
    Chats("Чаты"),
    Accounts("Аккаунты"),
    Relay("Relay"),
}

@Composable
fun LnlApp() {
    val context = androidx.compose.ui.platform.LocalContext.current.applicationContext
    val preferences = remember {
        RelayPreferences(context, BuildConfig.DEFAULT_BASE_URL)
    }
    val viewModel: LnlViewModel = viewModel(factory = LnlViewModel.factory(preferences))
    val state by viewModel.state.collectAsStateWithLifecycle()
    val snackbarHostState = remember { SnackbarHostState() }
    val lifecycleOwner = LocalLifecycleOwner.current
    var destination by rememberSaveable { mutableStateOf(HomeDestination.Chats) }

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
    BackHandler(enabled = state.activeChat == null && destination != HomeDestination.Chats) {
        destination = HomeDestination.Chats
    }

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            if (state.activeChat == null) {
                HomeTopBar(
                    destination = destination,
                    state = state,
                    onRefresh = viewModel::refreshChats,
                )
            } else {
                ChatTopBar(
                    state = state,
                    onBack = viewModel::closeChat,
                )
            }
        },
        bottomBar = {
            if (state.activeChat == null) {
                LnlBottomNavigation(
                    selected = destination,
                    onSelect = { destination = it },
                )
            }
        },
        snackbarHost = { SnackbarHost(snackbarHostState) },
    ) { padding ->
        if (state.activeChat != null) {
            ChatScreen(
                state = state,
                padding = padding,
                onDraftChange = viewModel::updateDraft,
                onSend = viewModel::sendMessage,
                onRetry = viewModel::retryMessages,
            )
        } else {
            when (destination) {
                HomeDestination.Chats -> ChatsScreen(
                    state = state,
                    padding = padding,
                    onRefresh = viewModel::refreshChats,
                    onOpenChat = viewModel::openChat,
                    onSelectSession = viewModel::selectSession,
                    onOpenRelay = { destination = HomeDestination.Relay },
                )

                HomeDestination.Accounts -> AccountsScreen(
                    state = state,
                    padding = padding,
                    onSelectSession = {
                        viewModel.selectSession(it)
                        destination = HomeDestination.Chats
                    },
                    onOpenRelay = { destination = HomeDestination.Relay },
                )

                HomeDestination.Relay -> RelayScreen(
                    state = state,
                    padding = padding,
                    allowCleartext = BuildConfig.ALLOW_CLEARTEXT,
                    onBaseUrlChange = viewModel::updateBaseUrlDraft,
                    onConnect = viewModel::connect,
                )
            }
        }
    }
}

@Composable
private fun HomeTopBar(
    destination: HomeDestination,
    state: LnlUiState,
    onRefresh: () -> Unit,
) {
    Column {
        Surface(
            color = MaterialTheme.colorScheme.surface,
            tonalElevation = 1.dp,
        ) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .statusBarsPadding()
                    .heightIn(min = 64.dp)
                    .padding(start = 16.dp, end = 4.dp, top = 6.dp, bottom = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Box(modifier = Modifier.weight(1f)) {
                    when (destination) {
                        HomeDestination.Chats -> {
                            Row(
                                horizontalArrangement = Arrangement.spacedBy(10.dp),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                InitialAvatar(
                                    label = state.selectedSessionId ?: "LNL",
                                    size = 40.dp,
                                )
                                Column {
                                    Text(
                                        text = "Чаты",
                                        maxLines = 1,
                                    )
                                    HeaderStatus(
                                        sessionId = state.selectedSessionId,
                                        connection = state.connection,
                                    )
                                }
                            }
                        }

                        HomeDestination.Accounts -> {
                            Column {
                                Text("Аккаунты")
                                Text(
                                    text = if (state.sessions.isEmpty()) {
                                        "Нет подключённых сессий"
                                    } else {
                                        "Доступно: ${state.sessions.size}"
                                    },
                                    style = MaterialTheme.typography.labelSmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }

                        HomeDestination.Relay -> {
                            Column {
                                Text("Подключение")
                                Text(
                                    text = state.connection.label(),
                                    style = MaterialTheme.typography.labelSmall,
                                    color = connectionColor(state.connection),
                                )
                            }
                        }
                    }
                }
                if (destination == HomeDestination.Chats) {
                    IconButton(
                        onClick = onRefresh,
                        enabled = state.selectedSessionId != null && !state.loadingChats,
                    ) {
                        Icon(
                            imageVector = Icons.Rounded.Refresh,
                            contentDescription = "Обновить диалоги",
                        )
                    }
                }
            }
        }
        if (state.loadingSessions || state.loadingChats) {
            LinearProgressIndicator(Modifier.fillMaxWidth())
        }
    }
}

@Composable
private fun ChatTopBar(
    state: LnlUiState,
    onBack: () -> Unit,
) {
    val chat = state.activeChat ?: return
    Column {
        Surface(
            color = MaterialTheme.colorScheme.surface,
            tonalElevation = 1.dp,
        ) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .statusBarsPadding()
                    .heightIn(min = 64.dp)
                    .padding(horizontal = 4.dp, vertical = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                IconButton(onClick = onBack) {
                    Icon(
                        imageVector = Icons.AutoMirrored.Rounded.ArrowBack,
                        contentDescription = "Назад к чатам",
                    )
                }
                Row(
                    modifier = Modifier.weight(1f),
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    InitialAvatar(
                        label = chat.title,
                        size = 40.dp,
                    )
                    Column {
                        Text(
                            text = chat.title,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        HeaderStatus(
                            sessionId = state.selectedSessionId,
                            connection = state.connection,
                        )
                    }
                }
            }
        }
        if (state.loadingMessages) {
            LinearProgressIndicator(Modifier.fillMaxWidth())
        }
    }
}

@Composable
private fun HeaderStatus(
    sessionId: String?,
    connection: ConnectionStatus,
) {
    Row(
        horizontalArrangement = Arrangement.spacedBy(5.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val palette = LocalLnlPalette.current
        Surface(
            modifier = Modifier.size(7.dp),
            shape = RoundedCornerShape(50),
            color = when (connection) {
                ConnectionStatus.Online -> palette.online
                ConnectionStatus.Connecting -> palette.warning
                ConnectionStatus.Offline -> MaterialTheme.colorScheme.error
                ConnectionStatus.Idle -> MaterialTheme.colorScheme.outline
            },
            content = {},
        )
        Text(
            text = buildString {
                append(sessionId ?: "аккаунт не выбран")
                append(" · ")
                append(connection.label())
            },
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun LnlBottomNavigation(
    selected: HomeDestination,
    onSelect: (HomeDestination) -> Unit,
) {
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .navigationBarsPadding()
            .padding(horizontal = 16.dp, vertical = 8.dp),
        contentAlignment = Alignment.Center,
    ) {
        Surface(
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(min = 64.dp),
            shape = RoundedCornerShape(28.dp),
            color = MaterialTheme.colorScheme.surface,
            shadowElevation = 8.dp,
            tonalElevation = 2.dp,
        ) {
            NavigationBar(
                containerColor = Color.Transparent,
                tonalElevation = 0.dp,
                windowInsets = WindowInsets(0, 0, 0, 0),
            ) {
                HomeDestination.entries.forEach { destination ->
                    NavigationBarItem(
                        selected = selected == destination,
                        onClick = { onSelect(destination) },
                        icon = {
                            Icon(
                                imageVector = destination.icon,
                                contentDescription = null,
                            )
                        },
                        label = {
                            Text(
                                text = destination.label,
                                maxLines = 1,
                            )
                        },
                    )
                }
            }
        }
    }
}

private val HomeDestination.icon: ImageVector
    get() = when (this) {
        HomeDestination.Chats -> Icons.AutoMirrored.Rounded.List
        HomeDestination.Accounts -> Icons.Rounded.Person
        HomeDestination.Relay -> Icons.Rounded.Settings
    }

@Composable
private fun connectionColor(status: ConnectionStatus): Color {
    val palette = LocalLnlPalette.current
    return when (status) {
        ConnectionStatus.Idle -> MaterialTheme.colorScheme.onSurfaceVariant
        ConnectionStatus.Connecting -> palette.warning
        ConnectionStatus.Online -> palette.online
        ConnectionStatus.Offline -> MaterialTheme.colorScheme.error
    }
}
