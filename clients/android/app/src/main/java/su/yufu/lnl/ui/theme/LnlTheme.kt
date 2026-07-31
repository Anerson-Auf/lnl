package su.yufu.lnl.ui.theme

import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp

private val LightColorScheme = lightColorScheme(
    primary = Color(0xFF147DAC),
    onPrimary = Color.White,
    primaryContainer = Color(0xFFDDF2FC),
    onPrimaryContainer = Color(0xFF00364D),
    secondary = Color(0xFF4D626D),
    onSecondary = Color.White,
    secondaryContainer = Color(0xFFDCEEF7),
    onSecondaryContainer = Color(0xFF0A3446),
    background = Color(0xFFF6F8FA),
    onBackground = Color(0xFF1A1D21),
    surface = Color.White,
    onSurface = Color(0xFF1A1D21),
    surfaceVariant = Color(0xFFF0F3F5),
    onSurfaceVariant = Color(0xFF5F666B),
    outline = Color(0xFFD6DCE0),
    outlineVariant = Color(0xFFE7EBEE),
    error = Color(0xFFB3261E),
)

private val DarkColorScheme = darkColorScheme(
    primary = Color(0xFF69C5F5),
    onPrimary = Color(0xFF003548),
    primaryContainer = Color(0xFF164D67),
    onPrimaryContainer = Color(0xFFC6E9FA),
    secondary = Color(0xFFB7C9D2),
    onSecondary = Color(0xFF22343C),
    secondaryContainer = Color(0xFF334A55),
    onSecondaryContainer = Color(0xFFD3E7F0),
    background = Color(0xFF0F161D),
    onBackground = Color(0xFFF1F1F1),
    surface = Color(0xFF181F26),
    onSurface = Color(0xFFF1F1F1),
    surfaceVariant = Color(0xFF252D34),
    onSurfaceVariant = Color(0xFFADB6BC),
    outline = Color(0xFF4B555C),
    outlineVariant = Color(0xFF303941),
    error = Color(0xFFFFB4AB),
)

@Immutable
data class LnlPalette(
    val chatBackground: Color,
    val incomingBubble: Color,
    val onIncomingBubble: Color,
    val outgoingBubble: Color,
    val onOutgoingBubble: Color,
    val online: Color,
    val warning: Color,
    val avatarColors: List<Color>,
)

private val LightPalette = LnlPalette(
    chatBackground = Color(0xFFE7EDF1),
    incomingBubble = Color.White,
    onIncomingBubble = Color(0xFF1A1D21),
    outgoingBubble = Color(0xFFEFFFDE),
    onOutgoingBubble = Color(0xFF142012),
    online = Color(0xFF137333),
    warning = Color(0xFF9A6700),
    avatarColors = listOf(
        Color(0xFF2F6FA4),
        Color(0xFF5368B0),
        Color(0xFF70539A),
        Color(0xFFA13F6A),
        Color(0xFFA94F2D),
        Color(0xFF2F7A58),
    ),
)

private val DarkPalette = LnlPalette(
    chatBackground = Color(0xFF0F171F),
    incomingBubble = Color(0xFF1F292F),
    onIncomingBubble = Color(0xFFF1F1F1),
    outgoingBubble = Color(0xFF2B5278),
    onOutgoingBubble = Color(0xFFF4F8FB),
    online = Color(0xFF56D364),
    warning = Color(0xFFE3B341),
    avatarColors = listOf(
        Color(0xFF2F6FA4),
        Color(0xFF445B9A),
        Color(0xFF604B93),
        Color(0xFF8C3E62),
        Color(0xFF914528),
        Color(0xFF2B6C4D),
    ),
)

val LocalLnlPalette = staticCompositionLocalOf { LightPalette }

private val DefaultTypography = Typography()
private val LnlTypography = Typography(
    titleLarge = DefaultTypography.titleLarge.copy(fontWeight = FontWeight.SemiBold),
    titleMedium = DefaultTypography.titleMedium.copy(fontWeight = FontWeight.SemiBold),
    titleSmall = DefaultTypography.titleSmall.copy(fontWeight = FontWeight.SemiBold),
)

private val LnlShapes = Shapes(
    small = RoundedCornerShape(10.dp),
    medium = RoundedCornerShape(16.dp),
    large = RoundedCornerShape(24.dp),
)

@Composable
fun LnlAppTheme(content: @Composable () -> Unit) {
    val darkTheme = isSystemInDarkTheme()
    val colorScheme = if (darkTheme) DarkColorScheme else LightColorScheme
    val palette = if (darkTheme) DarkPalette else LightPalette

    androidx.compose.runtime.CompositionLocalProvider(LocalLnlPalette provides palette) {
        MaterialTheme(
            colorScheme = colorScheme,
            typography = LnlTypography,
            shapes = LnlShapes,
            content = content,
        )
    }
}
