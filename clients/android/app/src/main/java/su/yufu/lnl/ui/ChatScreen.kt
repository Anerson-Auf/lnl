package su.yufu.lnl.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.AbsoluteRoundedCornerShape
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.Send
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.AbsoluteAlignment
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import java.text.DateFormat
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.util.Date
import java.util.Locale
import su.yufu.lnl.R
import su.yufu.lnl.data.MediaInfo
import su.yufu.lnl.data.Message
import su.yufu.lnl.ui.theme.LocalLnlPalette

@Composable
internal fun ChatScreen(
    state: LnlUiState,
    padding: PaddingValues,
    onDraftChange: (String) -> Unit,
    onSend: () -> Unit,
    onRetry: () -> Unit,
) {
    val palette = LocalLnlPalette.current
    val listState = rememberLazyListState()
    var previousMessageCount by rememberSaveable(state.activeChat?.peerId) {
        mutableIntStateOf(state.messages.size)
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
            .consumeWindowInsets(padding)
            .imePadding()
            .background(palette.chatBackground),
    ) {
        if (state.activeChat?.pinned == true) {
            PinnedBanner()
        }
        if (state.messagesError != null && state.messages.isNotEmpty()) {
            HistoryWarning(onRetry)
        }

        when {
            state.loadingMessages && state.messages.isEmpty() -> {
                MessageLoadingSkeleton(Modifier.weight(1f))
            }

            state.messagesError != null && state.messages.isEmpty() -> {
                EmptyState(
                    title = "История не загрузилась",
                    subtitle = state.messagesError,
                    modifier = Modifier.weight(1f),
                    actionLabel = "Повторить",
                    onAction = onRetry,
                )
            }

            state.messages.isEmpty() -> {
                EmptyState(
                    title = "Сообщений пока нет",
                    subtitle = "Напиши первое сообщение.",
                    modifier = Modifier.weight(1f),
                )
            }

            else -> {
                LazyColumn(
                    state = listState,
                    modifier = Modifier
                        .weight(1f)
                        .fillMaxWidth(),
                    contentPadding = PaddingValues(
                        start = 10.dp,
                        top = 10.dp,
                        end = 10.dp,
                        bottom = 8.dp,
                    ),
                    verticalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    itemsIndexed(
                        items = state.messages,
                        key = { _, message -> message.id },
                    ) { index, message ->
                        Column(
                            verticalArrangement = Arrangement.spacedBy(4.dp),
                        ) {
                            if (index == 0 || !message.isSameDay(state.messages[index - 1])) {
                                DateSeparator(message.dateLabel())
                            }
                            MessageBubble(message)
                        }
                    }
                }
            }
        }

        MessageComposer(
            draft = state.draft,
            sending = state.sending,
            onDraftChange = onDraftChange,
            onSend = onSend,
        )
    }
}

@Composable
private fun PinnedBanner() {
    Surface(
        color = MaterialTheme.colorScheme.surface,
        tonalElevation = 2.dp,
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(min = 42.dp)
                .padding(horizontal = 14.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                painter = painterResource(R.drawable.ic_push_pin),
                contentDescription = null,
                modifier = Modifier.size(18.dp),
                tint = MaterialTheme.colorScheme.primary,
            )
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = "Закреплённый диалог",
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.primary,
                )
                Text(
                    text = "Диалог закреплён в Telegram",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
private fun HistoryWarning(onRetry: () -> Unit) {
    Surface(
        color = MaterialTheme.colorScheme.errorContainer,
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(min = 44.dp)
                .padding(start = 14.dp, end = 6.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = "Не удалось обновить историю",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onErrorContainer,
            )
            TextButton(onClick = onRetry) {
                Text("Повторить")
            }
        }
    }
}

@Composable
private fun DateSeparator(label: String) {
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 6.dp),
        contentAlignment = Alignment.Center,
    ) {
        Surface(
            shape = RoundedCornerShape(12.dp),
            color = MaterialTheme.colorScheme.surface.copy(alpha = 0.92f),
            tonalElevation = 1.dp,
        ) {
            Text(
                text = label,
                modifier = Modifier.padding(horizontal = 10.dp, vertical = 4.dp),
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun MessageBubble(message: Message) {
    val palette = LocalLnlPalette.current
    val alignment = if (message.outgoing) {
        AbsoluteAlignment.CenterRight
    } else {
        AbsoluteAlignment.CenterLeft
    }
    val bubbleColor = if (message.outgoing) {
        palette.outgoingBubble
    } else {
        palette.incomingBubble
    }
    val contentColor = if (message.outgoing) {
        palette.onOutgoingBubble
    } else {
        palette.onIncomingBubble
    }
    val shape = if (message.outgoing) {
        AbsoluteRoundedCornerShape(
            topLeft = 18.dp,
            topRight = 18.dp,
            bottomLeft = 18.dp,
            bottomRight = 5.dp,
        )
    } else {
        AbsoluteRoundedCornerShape(
            topLeft = 18.dp,
            topRight = 18.dp,
            bottomLeft = 5.dp,
            bottomRight = 18.dp,
        )
    }

    BoxWithConstraints(Modifier.fillMaxWidth()) {
        Surface(
            modifier = Modifier
                .align(alignment)
                .widthIn(max = minOf(maxWidth * 0.84f, 560.dp))
                .semantics(mergeDescendants = true) {
                    stateDescription = if (message.outgoing) {
                        "Исходящее сообщение"
                    } else {
                        "Входящее сообщение"
                    }
                },
            color = bubbleColor,
            contentColor = contentColor,
            shape = shape,
            shadowElevation = 1.dp,
        ) {
            Column(
                modifier = Modifier.padding(horizontal = 11.dp, vertical = 8.dp),
                verticalArrangement = Arrangement.spacedBy(5.dp),
            ) {
                message.media?.let {
                    MediaSummary(
                        media = it,
                        contentColor = contentColor,
                    )
                }
                if (message.text.isNotBlank()) {
                    SelectionContainer {
                        Text(
                            text = message.text,
                            style = MaterialTheme.typography.bodyLarge,
                            color = contentColor,
                        )
                    }
                }
                Text(
                    text = message.timeLabel(),
                    style = MaterialTheme.typography.labelSmall,
                    color = contentColor.copy(alpha = 0.74f),
                    modifier = Modifier.align(Alignment.End),
                )
            }
        }
    }
}

@Composable
private fun MediaSummary(
    media: MediaInfo,
    contentColor: Color,
) {
    if (media.kind == "sticker" && !media.emoji.isNullOrBlank()) {
        Column(
            verticalArrangement = Arrangement.spacedBy(2.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text(
                text = media.emoji,
                fontSize = 42.sp,
            )
            Text(
                text = media.title(),
                style = MaterialTheme.typography.labelMedium,
                color = contentColor.copy(alpha = 0.72f),
            )
        }
        return
    }

    Surface(
        shape = RoundedCornerShape(13.dp),
        color = contentColor.copy(alpha = 0.08f),
        contentColor = contentColor,
    ) {
        Row(
            modifier = Modifier
                .widthIn(min = 190.dp, max = 280.dp)
                .padding(9.dp),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Surface(
                modifier = Modifier.size(44.dp),
                shape = if (media.kind == "video_note") CircleShape else RoundedCornerShape(11.dp),
                color = contentColor.copy(alpha = 0.12f),
                contentColor = contentColor,
            ) {
                Box(contentAlignment = Alignment.Center) {
                    Icon(
                        painter = painterResource(media.iconRes()),
                        contentDescription = null,
                        modifier = Modifier.size(24.dp),
                        tint = contentColor,
                    )
                }
            }
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                Text(
                    text = media.title(),
                    style = MaterialTheme.typography.titleSmall,
                    color = contentColor,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                val details = media.details()
                if (details.isNotBlank()) {
                    Text(
                        text = details,
                        style = MaterialTheme.typography.bodySmall,
                        color = contentColor.copy(alpha = 0.68f),
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
    }
}

@Composable
private fun MessageComposer(
    draft: String,
    sending: Boolean,
    onDraftChange: (String) -> Unit,
    onSend: () -> Unit,
) {
    Surface(
        color = MaterialTheme.colorScheme.surface,
        shadowElevation = 4.dp,
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 8.dp, vertical = 7.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.Bottom,
        ) {
            TextField(
                value = draft,
                onValueChange = onDraftChange,
                modifier = Modifier
                    .weight(1f)
                    .heightIn(min = 52.dp),
                placeholder = { Text("Сообщение") },
                minLines = 1,
                maxLines = 5,
                enabled = !sending,
                shape = RoundedCornerShape(26.dp),
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send),
                keyboardActions = KeyboardActions(onSend = { onSend() }),
                colors = TextFieldDefaults.colors(
                    focusedContainerColor = MaterialTheme.colorScheme.surfaceVariant,
                    unfocusedContainerColor = MaterialTheme.colorScheme.surfaceVariant,
                    disabledContainerColor = MaterialTheme.colorScheme.surfaceVariant,
                    focusedIndicatorColor = Color.Transparent,
                    unfocusedIndicatorColor = Color.Transparent,
                    disabledIndicatorColor = Color.Transparent,
                ),
            )
            FilledIconButton(
                onClick = onSend,
                enabled = draft.isNotBlank() && !sending,
                modifier = Modifier
                    .size(52.dp)
                    .semantics {
                        contentDescription = "Отправить сообщение"
                        if (sending) {
                            stateDescription = "Отправка"
                        }
                    },
                colors = IconButtonDefaults.filledIconButtonColors(
                    containerColor = MaterialTheme.colorScheme.primary,
                    contentColor = MaterialTheme.colorScheme.onPrimary,
                ),
            ) {
                if (sending) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(21.dp),
                        strokeWidth = 2.dp,
                        color = MaterialTheme.colorScheme.onPrimary,
                    )
                } else {
                    Icon(
                        imageVector = Icons.AutoMirrored.Rounded.Send,
                        contentDescription = null,
                    )
                }
            }
        }
    }
}

@Composable
private fun MessageLoadingSkeleton(modifier: Modifier = Modifier) {
    Column(
        modifier = modifier
            .fillMaxWidth()
            .padding(horizontal = 12.dp, vertical = 18.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        listOf(
            AbsoluteAlignment.Left to 0.58f,
            AbsoluteAlignment.Right to 0.68f,
            AbsoluteAlignment.Left to 0.76f,
            AbsoluteAlignment.Right to 0.44f,
        ).forEach { (alignment, widthFraction) ->
            Spacer(
                modifier = Modifier
                    .fillMaxWidth(widthFraction)
                    .height(54.dp)
                    .align(alignment)
                    .clip(RoundedCornerShape(18.dp))
                    .background(MaterialTheme.colorScheme.surface.copy(alpha = 0.72f)),
            )
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

private fun MediaInfo.iconRes(): Int = when (kind) {
    "sticker" -> R.drawable.ic_media_sticker
    "photo" -> R.drawable.ic_media_photo
    "file" -> R.drawable.ic_media_file
    "audio" -> R.drawable.ic_media_audio
    "video", "video_note" -> R.drawable.ic_media_video
    "voice" -> R.drawable.ic_media_voice
    else -> R.drawable.ic_media_file
}

private fun MediaInfo.details(): String = listOfNotNull(
    fileName,
    size?.let(::formatBytes),
    durationSeconds?.let(::formatDuration),
    if (width != null && height != null) "${width}×${height}" else null,
    mimeType,
    stickerFormat?.uppercase(Locale.ROOT),
    if (spoiler) "скрытое медиа" else null,
).distinct().joinToString(" · ")

private fun Message.isSameDay(other: Message): Boolean {
    val zone = ZoneId.systemDefault()
    val thisDay = Instant.ofEpochSecond(date.toLong()).atZone(zone).toLocalDate()
    val otherDay = Instant.ofEpochSecond(other.date.toLong()).atZone(zone).toLocalDate()
    return thisDay == otherDay
}

private fun Message.dateLabel(): String {
    val date = Instant
        .ofEpochSecond(date.toLong())
        .atZone(ZoneId.systemDefault())
        .toLocalDate()
    return DateTimeFormatter
        .ofLocalizedDate(FormatStyle.MEDIUM)
        .withLocale(Locale.getDefault())
        .format(date)
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
