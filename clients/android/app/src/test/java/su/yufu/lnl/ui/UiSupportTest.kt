package su.yufu.lnl.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import su.yufu.lnl.data.ChatSummary

class UiSupportTest {
    @Test
    fun searchMatchesRussianTitleAndPreviewIgnoringCase() {
        val chats = listOf(
            ChatSummary(peerId = 1, title = "Роман"),
            ChatSummary(peerId = 2, title = "Работа", lastMessage = "ГОТОВ НОВЫЙ МАКЕТ"),
        )

        assertEquals(listOf(1L), filterChats(chats, "ром", pinnedOnly = false).peerIds())
        assertEquals(listOf(2L), filterChats(chats, "новый макет", pinnedOnly = false).peerIds())
    }

    @Test
    fun pinnedFilterComposesWithSearch() {
        val chats = listOf(
            ChatSummary(peerId = 1, title = "Роман", pinned = true),
            ChatSummary(peerId = 2, title = "Роман — архив"),
            ChatSummary(peerId = 3, title = "Команда", pinned = true),
        )

        assertEquals(listOf(1L), filterChats(chats, "роман", pinnedOnly = true).peerIds())
        assertEquals(listOf(1L, 3L), filterChats(chats, "", pinnedOnly = true).peerIds())
    }

    @Test
    fun avatarInitialsUseReadableCharactersAndSafeFallback() {
        assertEquals("РЛ", avatarInitials("Роман Лучший"))
        assertEquals("A4", avatarInitials("account-42"))
        assertEquals("?", avatarInitials("✨"))
    }

    @Test
    fun avatarPaletteIndexHandlesMinimumHashValue() {
        val index = avatarPaletteIndex("polygenelubricants", paletteSize = 6)

        assertTrue(index in 0 until 6)
    }

    private fun List<ChatSummary>.peerIds(): List<Long> = map(ChatSummary::peerId)
}
