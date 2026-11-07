/*
 * Type.kt — typography. The engine console, pipeline art, and evidence hashes
 * are monospace `key:  value` lines (repo ASCII style), so Monospace is a
 * first-class style here, not an afterthought.
 */
package com.carryon.app.ui.theme

import androidx.compose.material3.Typography
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

val Mono = FontFamily.Monospace

/** Material 3 typography with monospace code/console styles layered on defaults. */
val CarryOnTypography: Typography = Typography()

/** Console line: `key:  value` monospace, small, tight. */
val ConsoleTextStyle = TextStyle(
    fontFamily = Mono,
    fontWeight = FontWeight.Normal,
    fontSize = 12.sp,
    lineHeight = 16.sp,
)

/** ASCII pipeline/banner art. */
val AsciiTextStyle = TextStyle(
    fontFamily = Mono,
    fontWeight = FontWeight.Medium,
    fontSize = 13.sp,
    lineHeight = 18.sp,
)

/** Big monospace banner (the CARRY-ON box onboarding). */
val BannerTextStyle = TextStyle(
    fontFamily = Mono,
    fontWeight = FontWeight.Bold,
    fontSize = 18.sp,
    lineHeight = 22.sp,
)
