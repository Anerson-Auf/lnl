package su.yufu.lnl.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import java.util.Locale
import su.yufu.lnl.data.ChatSummary
import su.yufu.lnl.ui.theme.LocalLnlPalette

@Composable
internal fun InitialAvatar(
    label: String,
    modifier: Modifier = Modifier,
    size: Dp = 52.dp,
    selected: Boolean = false,
) {
    val palette = LocalLnlPalette.current
    val color = palette.avatarColors[
        avatarPaletteIndex(label, palette.avatarColors.size)
    ]
    val semanticsModifier = modifier.clearAndSetSemantics {}
    Box(
        modifier = semanticsModifier
            .size(size)
            .background(
                color = if (selected) MaterialTheme.colorScheme.primary else color,
                shape = CircleShape,
            )
            .padding(if (selected) 2.dp else 0.dp)
            .background(
                color = if (selected) MaterialTheme.colorScheme.surface else color,
                shape = CircleShape,
            )
            .padding(if (selected) 2.dp else 0.dp)
            .background(color = color, shape = CircleShape),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = avatarInitials(label),
            color = androidx.compose.ui.graphics.Color.White,
            style = when {
                size >= 48.dp -> MaterialTheme.typography.titleMedium
                else -> MaterialTheme.typography.labelLarge
            },
            fontWeight = FontWeight.SemiBold,
            textAlign = TextAlign.Center,
        )
    }
}

internal fun filterChats(
    chats: List<ChatSummary>,
    query: String,
    pinnedOnly: Boolean,
): List<ChatSummary> {
    val normalizedQuery = query.trim()
    return chats.filter { chat ->
        (!pinnedOnly || chat.pinned) &&
            (
                normalizedQuery.isEmpty() ||
                    chat.title.contains(normalizedQuery, ignoreCase = true) ||
                    chat.lastMessage?.contains(normalizedQuery, ignoreCase = true) == true
                )
    }
}

internal fun avatarInitials(label: String): String {
    val tokens = label
        .trim()
        .split(Regex("[\\s._-]+"))
        .mapNotNull { token -> token.firstOrNull(Char::isLetterOrDigit) }

    val initials = when {
        tokens.size >= 2 -> tokens.take(2).joinToString("")
        else -> label.filter(Char::isLetterOrDigit).take(2)
    }
    return initials.ifBlank { "?" }.uppercase(Locale.ROOT)
}

internal fun avatarPaletteIndex(label: String, paletteSize: Int): Int {
    require(paletteSize > 0)
    return Math.floorMod(label.hashCode(), paletteSize)
}

internal fun ConnectionStatus.label(): String = when (this) {
    ConnectionStatus.Idle -> "relay не подключён"
    ConnectionStatus.Connecting -> "relay подключается"
    ConnectionStatus.Online -> "relay подключён"
    ConnectionStatus.Offline -> "relay недоступен"
}
