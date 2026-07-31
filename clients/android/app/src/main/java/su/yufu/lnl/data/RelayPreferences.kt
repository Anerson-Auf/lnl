package su.yufu.lnl.data

import android.content.Context

data class RelaySettings(
    val baseUrl: String,
    val sessionId: String?,
)

interface RelaySettingsStore {
    fun load(): RelaySettings
    fun saveBaseUrl(baseUrl: String)
    fun saveSessionId(sessionId: String)
}

class RelayPreferences(
    context: Context,
    defaultBaseUrl: String,
) : RelaySettingsStore {
    private val preferences = context.getSharedPreferences(FILE_NAME, Context.MODE_PRIVATE)
    private val fallbackBaseUrl = defaultBaseUrl

    override fun load(): RelaySettings = RelaySettings(
        baseUrl = preferences.getString(KEY_BASE_URL, null)
            ?.takeIf(String::isNotBlank)
            ?: fallbackBaseUrl,
        sessionId = preferences.getString(KEY_SESSION_ID, null)
            ?.takeIf(String::isNotBlank),
    )

    override fun saveBaseUrl(baseUrl: String) {
        preferences.edit().putString(KEY_BASE_URL, baseUrl).apply()
    }

    override fun saveSessionId(sessionId: String) {
        preferences.edit().putString(KEY_SESSION_ID, sessionId).apply()
    }

    private companion object {
        const val FILE_NAME = "relay"
        const val KEY_BASE_URL = "base_url"
        const val KEY_SESSION_ID = "session_id"
    }
}
