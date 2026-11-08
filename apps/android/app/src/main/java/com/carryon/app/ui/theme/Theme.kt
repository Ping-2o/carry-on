/*
 * Theme.kt — Material 3 theme for Carry-On.
 *
 * A complete, hand-tuned brand scheme (light + dark, every M3 role including
 * the surface-container ladder) generated from the Carry-On teal seed, plus
 * semantic status colors that M3 does not define (success / warning).
 *
 * Dynamic color (Material You) is OFF by default so the product looks the same
 * on every device and in CI screenshots; it can be switched on per call site.
 */
package com.carryon.app.ui.theme

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp

val LightScheme: ColorScheme = lightColorScheme(
    primary = Color(0xFF006C51),
    onPrimary = Color(0xFFFFFFFF),
    primaryContainer = Color(0xFF8CF5CF),
    onPrimaryContainer = Color(0xFF002117),
    secondary = Color(0xFF4B635A),
    onSecondary = Color(0xFFFFFFFF),
    secondaryContainer = Color(0xFFCDE9DC),
    onSecondaryContainer = Color(0xFF072019),
    tertiary = Color(0xFF3F6375),
    onTertiary = Color(0xFFFFFFFF),
    tertiaryContainer = Color(0xFFC3E8FD),
    onTertiaryContainer = Color(0xFF001F2A),
    error = Color(0xFFBA1A1A),
    onError = Color(0xFFFFFFFF),
    errorContainer = Color(0xFFFFDAD6),
    onErrorContainer = Color(0xFF410002),
    background = Color(0xFFF5FBF7),
    onBackground = Color(0xFF171D1A),
    surface = Color(0xFFF5FBF7),
    onSurface = Color(0xFF171D1A),
    surfaceVariant = Color(0xFFDBE5DF),
    onSurfaceVariant = Color(0xFF3F4945),
    surfaceTint = Color(0xFF006C51),
    inverseSurface = Color(0xFF2C322F),
    inverseOnSurface = Color(0xFFECF2EE),
    inversePrimary = Color(0xFF70D9B4),
    outline = Color(0xFF6F7975),
    outlineVariant = Color(0xFFBFC9C3),
    scrim = Color(0xFF000000),
    surfaceBright = Color(0xFFF5FBF7),
    surfaceDim = Color(0xFFD6DBD8),
    surfaceContainerLowest = Color(0xFFFFFFFF),
    surfaceContainerLow = Color(0xFFEFF5F1),
    surfaceContainer = Color(0xFFE9EFEB),
    surfaceContainerHigh = Color(0xFFE4EAE6),
    surfaceContainerHighest = Color(0xFFDEE4E0),
)

val DarkScheme: ColorScheme = darkColorScheme(
    primary = Color(0xFF70D9B4),
    onPrimary = Color(0xFF003828),
    primaryContainer = Color(0xFF00513D),
    onPrimaryContainer = Color(0xFF8CF5CF),
    secondary = Color(0xFFB2CCC1),
    onSecondary = Color(0xFF1D352D),
    secondaryContainer = Color(0xFF344C43),
    onSecondaryContainer = Color(0xFFCDE9DC),
    tertiary = Color(0xFFA7CCE1),
    onTertiary = Color(0xFF0B3545),
    tertiaryContainer = Color(0xFF264B5D),
    onTertiaryContainer = Color(0xFFC3E8FD),
    error = Color(0xFFFFB4AB),
    onError = Color(0xFF690005),
    errorContainer = Color(0xFF93000A),
    onErrorContainer = Color(0xFFFFDAD6),
    background = Color(0xFF0F1512),
    onBackground = Color(0xFFDEE4E0),
    surface = Color(0xFF0F1512),
    onSurface = Color(0xFFDEE4E0),
    surfaceVariant = Color(0xFF3F4945),
    onSurfaceVariant = Color(0xFFBFC9C3),
    surfaceTint = Color(0xFF70D9B4),
    inverseSurface = Color(0xFFDEE4E0),
    inverseOnSurface = Color(0xFF2C322F),
    inversePrimary = Color(0xFF006C51),
    outline = Color(0xFF89938E),
    outlineVariant = Color(0xFF3F4945),
    scrim = Color(0xFF000000),
    surfaceBright = Color(0xFF353B38),
    surfaceDim = Color(0xFF0F1512),
    surfaceContainerLowest = Color(0xFF0A0F0D),
    surfaceContainerLow = Color(0xFF171D1A),
    surfaceContainer = Color(0xFF1B211E),
    surfaceContainerHigh = Color(0xFF252B28),
    surfaceContainerHighest = Color(0xFF303633),
)

/** Semantic status colors (M3 has no success/warning roles). */
@Immutable
data class StatusColors(
    val success: Color,
    val onSuccess: Color,
    val successContainer: Color,
    val onSuccessContainer: Color,
    val warning: Color,
    val onWarning: Color,
    val warningContainer: Color,
    val onWarningContainer: Color,
)

private val LightStatus = StatusColors(
    success = Color(0xFF1B6D2F),
    onSuccess = Color(0xFFFFFFFF),
    successContainer = Color(0xFFA4F6A8),
    onSuccessContainer = Color(0xFF002107),
    warning = Color(0xFF7C5800),
    onWarning = Color(0xFFFFFFFF),
    warningContainer = Color(0xFFFFDEA6),
    onWarningContainer = Color(0xFF271900),
)

private val DarkStatus = StatusColors(
    success = Color(0xFF89D98E),
    onSuccess = Color(0xFF003911),
    successContainer = Color(0xFF00531C),
    onSuccessContainer = Color(0xFFA4F6A8),
    warning = Color(0xFFF7BD48),
    onWarning = Color(0xFF412D00),
    warningContainer = Color(0xFF5E4200),
    onWarningContainer = Color(0xFFFFDEA6),
)

val LocalStatusColors = staticCompositionLocalOf { LightStatus }

/** Generous, friendly corner radii (M3 shape scale). */
val CarryOnShapes = Shapes(
    extraSmall = RoundedCornerShape(6.dp),
    small = RoundedCornerShape(10.dp),
    medium = RoundedCornerShape(16.dp),
    large = RoundedCornerShape(24.dp),
    extraLarge = RoundedCornerShape(32.dp),
)

@Composable
fun CarryOnTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    // Material You is opt-in: brand colors by default (stable screenshots).
    dynamicColor: Boolean = false,
    content: @Composable () -> Unit,
) {
    val colorScheme = when {
        dynamicColor && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S -> {
            val context = LocalContext.current
            if (darkTheme) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        }
        darkTheme -> DarkScheme
        else -> LightScheme
    }
    CompositionLocalProvider(LocalStatusColors provides if (darkTheme) DarkStatus else LightStatus) {
        MaterialTheme(
            colorScheme = colorScheme,
            typography = CarryOnTypography,
            shapes = CarryOnShapes,
            content = content,
        )
    }
}
