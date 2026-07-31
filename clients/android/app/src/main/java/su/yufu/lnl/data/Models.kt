package su.yufu.lnl.data

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
data class ChatSummary(
    @SerialName("peer_id") val peerId: Long,
    val title: String,
    @SerialName("last_message") val lastMessage: String? = null,
)

@Serializable
data class Message(
    val id: Int,
    val text: String,
    val outgoing: Boolean,
    val date: Int,
)

@Serializable
data class SendBody(val text: String)

@Serializable
data class SendResponse(
    val ok: Boolean,
    val message: Message,
)

@Serializable
data class ErrorBody(val error: String)

@Serializable
data class WsEvent(
    val type: String,
    @SerialName("peer_id") val peerId: Long? = null,
    val message: Message? = null,
)
