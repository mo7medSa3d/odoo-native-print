package com.company.odoonativeprint.ui.theme

import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color

/**
 * Exact design tokens mirrored from desktop app/dist/theme.css (Light + Dark).
 */
@Immutable
data class ItbaaColorPalette(
    val isDark: Boolean,
    val bgCanvas: Color,
    val bgSurface: Color,
    val bgElevated: Color,
    val bgSubtle: Color,
    val bgHover: Color,
    val border: Color,
    val borderStrong: Color,
    val textPrimary: Color,
    val textSecondary: Color,
    val textMuted: Color,
    val brand: Color,
    val brandHover: Color,
    val brandText: Color,
    val okFg: Color,
    val okBg: Color,
    val warnFg: Color,
    val warnBg: Color,
    val errFg: Color,
    val errBg: Color,
    val destructive: Color,
    val danger: Color,
)

val ItbaaLightColors = ItbaaColorPalette(
    isDark = false,
    bgCanvas = Color(0xFFFCFCFC),
    bgSurface = Color(0xFFFFFFFF),
    bgElevated = Color(0xFFFAFAFA),
    bgSubtle = Color(0xFFF3F4F6),
    bgHover = Color(0xFFEAEDF2),
    border = Color(0xFFE7E4E4),
    borderStrong = Color(0xFF858894),
    textPrimary = Color(0xFF0E142B),
    textSecondary = Color(0xFF2D344D),
    textMuted = Color(0xFF555C72),
    brand = Color(0xFF2E67FF),
    brandHover = Color(0xFF1D54E8),
    brandText = Color(0xFF1E54E6),
    okFg = Color(0xFF046C4E),
    okBg = Color(0xFFE6F7F1),
    warnFg = Color(0xFF8C4A00),
    warnBg = Color(0xFFFFF4E5),
    errFg = Color(0xFFB8370A),
    errBg = Color(0xFFFEECE6),
    destructive = Color(0xFFFF6C35),
    danger = Color(0xFFC43D0E),
)

val ItbaaDarkColors = ItbaaColorPalette(
    isDark = true,
    bgCanvas = Color(0xFF000000),
    bgSurface = Color(0xFF0F0F0F),
    bgElevated = Color(0xFF141418),
    bgSubtle = Color(0xFF181818),
    bgHover = Color(0xFF212432),
    border = Color(0xFF26272E),
    borderStrong = Color(0xFF6B728E),
    textPrimary = Color(0xFFFFFFFF),
    textSecondary = Color(0xFFD9D9D9),
    textMuted = Color(0xFF9397A8),
    brand = Color(0xFF2B65FF),
    brandHover = Color(0xFF1D54E8),
    brandText = Color(0xFF6B96FF),
    okFg = Color(0xFF00EEBA),
    okBg = Color(0xFF082B24),
    warnFg = Color(0xFFF4EA55),
    warnBg = Color(0xFF2E2A0B),
    errFg = Color(0xFFFF8559),
    errBg = Color(0xFF331810),
    destructive = Color(0xFFFF6C35),
    danger = Color(0xFFC83E10),
)

val LocalItbaaColors = staticCompositionLocalOf { ItbaaLightColors }
