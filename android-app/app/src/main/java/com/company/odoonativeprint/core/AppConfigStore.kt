package com.company.odoonativeprint.core

import android.content.Context
import android.content.SharedPreferences
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.json.JSONArray
import org.json.JSONObject

data class AppConfigSnapshot(
    val origin: String? = null,
    val autoReprint: Boolean = false,
    val themeMode: String = "system",
    val language: String = "ar",
    val defaultPrinter: String? = null,
    val manualPrinters: List<ManualPrinterEntry> = emptyList(),
    val printerSettings: Map<String, PrinterSettings> = emptyMap(),
)

/**
 * Private persistent configuration store for origin, appearance, manual printers,
 * and per-printer overrides.
 */
class AppConfigStore(context: Context) {

    private val prefs: SharedPreferences =
        context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    private val _state = MutableStateFlow(loadSnapshot())
    val state: StateFlow<AppConfigSnapshot> = _state.asStateFlow()

    @Volatile
    var lastEnvironmentReport: String? = null

    fun current(): AppConfigSnapshot = _state.value

    fun saveOrigin(rawOrigin: String, autoReprint: Boolean): Result<String> {
        val validated = OriginPolicy.validateAndNormalize(rawOrigin)
        val normalized = validated.getOrElse { return Result.failure(it) }
        prefs.edit()
            .putString(KEY_ORIGIN, normalized)
            .putBoolean(KEY_AUTO_REPRINT, autoReprint)
            .apply()
        _state.value = _state.value.copy(origin = normalized, autoReprint = autoReprint)
        return Result.success(normalized)
    }

    fun setThemeMode(mode: String) {
        val safe = if (mode in setOf("system", "light", "dark")) mode else "system"
        prefs.edit().putString(KEY_THEME, safe).apply()
        _state.value = _state.value.copy(themeMode = safe)
    }

    fun setLanguage(lang: String) {
        val safe = if (lang == "en") "en" else "ar"
        prefs.edit().putString(KEY_LANG, safe).apply()
        _state.value = _state.value.copy(language = safe)
    }

    fun setDefaultPrinter(printerName: String?) {
        prefs.edit().putString(KEY_DEFAULT_PRINTER, printerName).apply()
        _state.value = _state.value.copy(defaultPrinter = printerName)
    }

    fun upsertManualPrinter(entry: ManualPrinterEntry) {
        val updated = _state.value.manualPrinters
            .filterNot { it.name.equals(entry.name, ignoreCase = true) } + entry
        val arr = JSONArray(updated.map { it.toJson() })
        prefs.edit().putString(KEY_MANUAL_PRINTERS, arr.toString()).apply()
        _state.value = _state.value.copy(manualPrinters = updated)
    }

    fun removeManualPrinter(name: String) {
        val updated = _state.value.manualPrinters
            .filterNot { it.name.equals(name, ignoreCase = true) }
        val arr = JSONArray(updated.map { it.toJson() })
        prefs.edit().putString(KEY_MANUAL_PRINTERS, arr.toString()).apply()
        _state.value = _state.value.copy(manualPrinters = updated)
    }

    fun getPrinterSettings(printerName: String): PrinterSettings =
        _state.value.printerSettings[printerName] ?: PrinterSettings()

    fun setPrinterSettings(printerName: String, settings: PrinterSettings) {
        val updated = _state.value.printerSettings.toMutableMap()
        updated[printerName] = settings
        val root = JSONObject()
        updated.forEach { (k, v) -> root.put(k, v.toJson()) }
        prefs.edit().putString(KEY_PRINTER_SETTINGS, root.toString()).apply()
        _state.value = _state.value.copy(printerSettings = updated)
    }

    private fun loadSnapshot(): AppConfigSnapshot {
        val manual = mutableListOf<ManualPrinterEntry>()
        prefs.getString(KEY_MANUAL_PRINTERS, null)?.let { raw ->
            runCatching {
                val arr = JSONArray(raw)
                for (i in 0 until arr.length()) {
                    manual += ManualPrinterEntry.fromJson(arr.getJSONObject(i))
                }
            }
        }
        val settingsMap = mutableMapOf<String, PrinterSettings>()
        prefs.getString(KEY_PRINTER_SETTINGS, null)?.let { raw ->
            runCatching {
                val obj = JSONObject(raw)
                val keys = obj.keys()
                while (keys.hasNext()) {
                    val key = keys.next()
                    settingsMap[key] = PrinterSettings.fromJson(obj.optJSONObject(key))
                }
            }
        }
        return AppConfigSnapshot(
            origin = prefs.getString(KEY_ORIGIN, null),
            autoReprint = prefs.getBoolean(KEY_AUTO_REPRINT, false),
            themeMode = prefs.getString(KEY_THEME, "system") ?: "system",
            language = prefs.getString(KEY_LANG, "ar") ?: "ar",
            defaultPrinter = prefs.getString(KEY_DEFAULT_PRINTER, null),
            manualPrinters = manual,
            printerSettings = settingsMap,
        )
    }

    companion object {
        private const val PREFS_NAME = "yaseir_print_config"
        private const val KEY_ORIGIN = "origin"
        private const val KEY_AUTO_REPRINT = "auto_reprint"
        private const val KEY_THEME = "theme_mode"
        private const val KEY_LANG = "language"
        private const val KEY_DEFAULT_PRINTER = "default_printer"
        private const val KEY_MANUAL_PRINTERS = "manual_printers"
        private const val KEY_PRINTER_SETTINGS = "printer_settings"
    }
}
