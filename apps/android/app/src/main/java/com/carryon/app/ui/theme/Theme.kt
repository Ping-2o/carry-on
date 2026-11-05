/*
 * Theme.kt — Material 3 with dynamic color on API 31+, dark theme default
 * (the repo ASCII art is designed on a dark terminal), light theme supported.
 */
package com.carryon.app.ui.theme

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext

private val DarkScheme = darkColorScheme(
    primary = Color(0xFF7FD1B9),
    secondary = Color(0xFF9BB0C9),
    tertiary = Color(0xFFD1A97F),
    background = Color(0xFF0B0E11),
    surface = Color(0xFF11161B),
    surfaceVariant = Color(0xFF1A2129),
)

private val LightScheme = lightColorScheme(
    primary = Color(0xFF2A6B58),
    secondary = Color(0xFF4A5B70),
    tertiary = Color(0xFF7A5A34),
)

@Composable
fun CarryOnTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    // Material You dynamic color where the platform offers it (API 31+).
    dynamicColor: Boolean = true,
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
    MaterialTheme(
        colorScheme = colorScheme,
        typography = CarryOnTypography,
        content = content,
    )
}
