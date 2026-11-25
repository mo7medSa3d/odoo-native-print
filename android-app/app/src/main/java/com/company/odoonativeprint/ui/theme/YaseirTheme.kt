package com.company.odoonativeprint.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.unit.LayoutDirection

@Composable
fun YaseirTheme(
    themeMode: String = "system",
    language: String = "ar",
    content: @Composable () -> Unit,
) {
    val isDark = when (themeMode) {
        "dark" -> true
        "light" -> false
        else -> isSystemInDarkTheme()
    }
    val palette = if (isDark) YaseirDarkColors else YaseirLightColors
    val layoutDirection = if (language == "en") LayoutDirection.Ltr else LayoutDirection.Rtl

    val materialScheme = if (isDark) {
        darkColorScheme(
            primary = palette.brand,
            onPrimary = Color.White,
            background = palette.bgCanvas,
            onBackground = palette.textPrimary,
            surface = palette.bgSurface,
            onSurface = palette.textPrimary,
            surfaceVariant = palette.bgElevated,
            onSurfaceVariant = palette.textSecondary,
            outline = palette.border,
            error = palette.errFg,
        )
    } else {
        lightColorScheme(
            primary = palette.brand,
            onPrimary = Color.White,
            background = palette.bgCanvas,
            onBackground = palette.textPrimary,
            surface = palette.bgSurface,
            onSurface = palette.textPrimary,
            surfaceVariant = palette.bgElevated,
            onSurfaceVariant = palette.textSecondary,
            outline = palette.border,
            error = palette.errFg,
        )
    }

    CompositionLocalProvider(
        LocalYaseirColors provides palette,
        LocalLayoutDirection provides layoutDirection,
    ) {
        MaterialTheme(
            colorScheme = materialScheme,
            typography = YaseirTypography,
            content = content,
        )
    }
}
