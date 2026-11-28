package com.company.odoonativeprint.ui.theme

import android.app.Activity
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.unit.LayoutDirection
import androidx.core.view.WindowInsetsControllerCompat

@Composable
fun ItbaaTheme(
    themeMode: String = "system",
    language: String = "ar",
    content: @Composable () -> Unit,
) {
    val isDark = when (themeMode) {
        "dark" -> true
        "light" -> false
        else -> isSystemInDarkTheme()
    }
    val palette = if (isDark) ItbaaDarkColors else ItbaaLightColors
    val layoutDirection = if (language == "en") LayoutDirection.Ltr else LayoutDirection.Rtl

    // Edge-to-edge leaves the status bar transparent. Dark icons are required
    // on a light background, otherwise the clock/battery/icons disappear.
    val activity = LocalContext.current as? Activity
    LaunchedEffect(isDark) {
        activity?.window?.let { window ->
            val controller = WindowInsetsControllerCompat(window, window.decorView)
            controller.isAppearanceLightStatusBars = !isDark
            controller.isAppearanceLightNavigationBars = !isDark
        }
    }

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
        LocalItbaaColors provides palette,
        LocalLayoutDirection provides layoutDirection,
        LocalLanguage provides language,
    ) {
        MaterialTheme(
            colorScheme = materialScheme,
            typography = ItbaaTypography,
            content = content,
        )
    }
}
