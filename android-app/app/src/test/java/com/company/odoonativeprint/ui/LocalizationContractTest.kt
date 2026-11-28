package com.company.odoonativeprint.ui

import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import java.io.File

class LocalizationContractTest {
    private fun source(relative: String): String {
        val tail = "app/src/main/java/com/company/odoonativeprint/$relative"
        return listOf(File(tail), File("../$tail"), File("../../$tail"))
            .first { it.isFile }
            .readText()
    }

    @Test
    fun `compose chrome has no untranslated Arabic literals`() {
        val arabic = Regex("[\\u0600-\\u06FF]")
        listOf(
            "ui/settings/SettingsScreen.kt",
            "ui/controlcenter/ControlCenterScreen.kt",
            "ui/controlcenter/PrinterConfigSheet.kt",
            "ui/browser/BootScreen.kt",
            "ui/browser/BrowserScreen.kt",
        ).forEach { file ->
            val unwrapped = source(file).lines().filter { line ->
                arabic.containsMatchIn(line) && !line.contains("tr(\"") && !line.contains("localize(\"")
            }
            assertTrue(unwrapped.isEmpty(), "$file has unwrapped Arabic: $unwrapped")
        }
    }

    @Test
    fun `language preference is wired into composition and system bars`() {
        val theme = source("ui/theme/ItbaaTheme.kt")
        assertTrue(theme.contains("LocalLanguage provides language"))
        assertTrue(theme.contains("isAppearanceLightStatusBars = !isDark"))
    }
}
