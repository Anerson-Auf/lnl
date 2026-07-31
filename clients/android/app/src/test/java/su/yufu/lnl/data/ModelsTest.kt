package su.yufu.lnl.data

import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ModelsTest {
    private val json = Json { ignoreUnknownKeys = true }

    @Test
    fun decodesScopedSessionsAndOptionalPinState() {
        val sessions = json.decodeFromString<List<SessionSummary>>(
            """[{"id":"default","is_default":true},{"id":"work","is_default":false}]""",
        )
        val chats = json.decodeFromString<List<ChatSummary>>(
            """[
                {"peer_id":1,"title":"Pinned","pinned":true},
                {"peer_id":2,"title":"Legacy","last_message":null}
            ]""".trimIndent(),
        )

        assertEquals("default", sessions.first().id)
        assertTrue(sessions.first().isDefault)
        assertTrue(chats.first().pinned)
        assertFalse(chats.last().pinned)
    }

    @Test
    fun decodesMediaMessagesAndPinEventsWithoutBreakingLegacyText() {
        val mediaMessage = json.decodeFromString<Message>(
            """{
                "id":45,
                "text":"",
                "outgoing":false,
                "date":1710000003,
                "media":{
                    "kind":"voice",
                    "mime_type":"audio/ogg",
                    "size":18240,
                    "duration_seconds":7,
                    "downloadable":true,
                    "spoiler":false
                }
            }""".trimIndent(),
        )
        val legacyMessage = json.decodeFromString<Message>(
            """{"id":42,"text":"привет","outgoing":false,"date":1710000000}""",
        )
        val pinEvent = json.decodeFromString<WsEvent>(
            """{"type":"chat_pinned","peer_id":123,"pinned":true}""",
        )

        assertEquals("voice", mediaMessage.media?.kind)
        assertEquals(7, mediaMessage.media?.durationSeconds)
        assertNull(legacyMessage.media)
        assertTrue(pinEvent.pinned == true)
    }
}
