package com.company.odoonativeprint.core

import android.content.Context
import android.content.Intent
import android.os.Build
import androidx.core.content.FileProvider
import com.company.odoonativeprint.BuildConfig
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/**
 * Generates a redacted JSON diagnostics bundle and shares it via FileProvider.
 * Never exports cookies, session tokens, or raw document payloads.
 */
object DiagnosticsExporter {

    fun exportAndShare(
        context: Context,
        config: AppConfigStore,
        queue: JobQueueEngine,
        printers: List<PrinterInfo>,
    ): File {
        val dir = File(context.cacheDir, "diagnostics").apply { mkdirs() }
        val file = File(dir, "yaseir-print-diagnostics-${System.currentTimeMillis()}.json")

        val snap = config.current()
        val payload = JSONObject().apply {
            put("app_version", BuildConfig.VERSION_NAME)
            put("platform", "android")
            put("android_sdk", Build.VERSION.SDK_INT)
            put("device_model", "${Build.MANUFACTURER} ${Build.MODEL}")
            put("configured_origin", snap.origin ?: JSONObject.NULL)
            put("auto_reprint", snap.autoReprint)
            put("theme_mode", snap.themeMode)
            put("language", snap.language)
            put("printers", JSONArray(printers.map { it.toJson() }))
            put("queue_summary", queue.summary().toJson())
            put("recent_jobs", JSONArray(queue.listJobs(50).map { it.toJson() }))
            config.lastEnvironmentReport?.let { raw ->
                runCatching { put("webview_environment", JSONObject(raw)) }
            }
        }

        file.writeText(payload.toString(2), Charsets.UTF_8)

        val uri = FileProvider.getUriForFile(
            context,
            "${context.packageName}.fileprovider",
            file,
        )
        val shareIntent = Intent(Intent.ACTION_SEND).apply {
            type = "application/json"
            putExtra(Intent.EXTRA_STREAM, uri)
            putExtra(Intent.EXTRA_SUBJECT, "Yaseir Print Android Diagnostics")
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        }
        context.startActivity(Intent.createChooser(shareIntent, "تصدير حزمة التشخيص").addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        return file
    }
}
