package su.yufu.lnl.data

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
data class SessionSummary(
    val id: String,
    @SerialName("is_default") val isDefault: Boolean,
)

@Serializable
data class ChatSummary(
    @SerialName("peer_id") val peerId: Long,
    val title: String,
    @SerialName("last_message") val lastMessage: String? = null,
    val pinned: Boolean = false,
)

@Serializable
data class Message(
    val id: Int,
    val text: String,
    val outgoing: Boolean,
    val date: Int,
    val media: MediaInfo? = null,
)

@Serializable
data class MediaInfo(
    val kind: String,
    @SerialName("mime_type") val mimeType: String? = null,
    val size: Long? = null,
    @SerialName("file_name") val fileName: String? = null,
    @SerialName("duration_seconds") val durationSeconds: Int? = null,
    val width: Int? = null,
    val height: Int? = null,
    val emoji: String? = null,
    @SerialName("sticker_format") val stickerFormat: String? = null,
    val downloadable: Boolean = false,
    val spoiler: Boolean = false,
)

@Serializable
data class SendBody(val text: String)

@Serializable
data class SendResponse(
    val ok: Boolean,
    val message: Message,
)

@Serializable
data class ErrorBody(
    val error: String,
    val code: String? = null,
)

@Serializable
data class WsEvent(
    val type: String,
    @SerialName("peer_id") val peerId: Long? = null,
    val message: Message? = null,
    val pinned: Boolean? = null,
)

sealed interface SocketEvent {
    data object Connected : SocketEvent
    data class Payload(val event: WsEvent) : SocketEvent
}
