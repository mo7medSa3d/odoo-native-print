package com.company.odoonativeprint.bridge

import android.util.Base64
import android.webkit.WebView
import androidx.webkit.JavaScriptReplyProxy
import androidx.webkit.WebMessageCompat
import androidx.webkit.WebViewCompat
import com.company.odoonativeprint.core.AppOverview
import com.company.odoonativeprint.core.OriginPolicy
import com.company.odoonativeprint.core.PayloadLimits
import com.company.odoonativeprint.core.PrintJobRecord
import com.company.odoonativeprint.core.PrinterCapabilities
import com.company.odoonativeprint.core.PrinterInfo
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject

/**
 * Pure-JVM command handler surface for the 13 remote-allowed commands.
 * Admin-only actions are intentionally absent from this interface (ADR-0005).
 */
interface RemoteBridgeHandler {
    fun configuredOrigin(): String?
    fun enqueuePdf(printer: String, bytes: ByteArray, copies: Int, idempotencyKey: String?, options: JSONObject): PrintJobRecord
    fun enqueueText(printer: String, text: String, copies: Int, idempotencyKey: String?, options: JSONObject): PrintJobRecord
    fun enqueueImage(printer: String, bytes: ByteArray, copies: Int, idempotencyKey: String?, options: JSONObject): PrintJobRecord
    fun printStatus(jobId: String): String?
    fun jobDetail(jobId: String): PrintJobRecord?
    fun appOverview(force: Boolean): AppOverview
    fun listPrinters(force: Boolean): List<PrinterInfo>
    fun printerForms(printer: String): List<String>
    fun printerCapabilities(printer: String): PrinterCapabilities?
    fun openControlCenter()
    fun openSettings()
    fun openTab(url: String)
    fun reportEnvironment(report: String)
}

/**
 * Security-enforcing dispatcher for WebView JS bridge calls.
 * Strictly checks:
 * 1. isMainFrame == true (blocks third-party iframes)
 * 2. OriginPolicy.isSameOrigin(configuredOrigin, sourceOrigin)
 * 3. Command is strictly within the 13 remote-allowed commands (blocks all local admin commands).
 */
class BridgeDispatcher(
    private val handler: RemoteBridgeHandler,
    private val decodeBase64: (String) -> ByteArray = { Base64.decode(it, Base64.DEFAULT) },
) {
    fun dispatch(
        sourceOrigin: String,
        isMainFrame: Boolean,
        rawMessage: String,
    ): JSONObject {
        val req = try {
            JSONObject(rawMessage)
        } catch (_: Exception) {
            return err("", "ERR_MALFORMED_JSON", "Invalid bridge payload")
        }
        val id = req.optString("id", "")
        val cmd = req.optString("cmd", "")
        val args = req.optJSONObject("args") ?: JSONObject()

        if (!isMainFrame) {
            return err(id, "ERR_SUBFRAME_FORBIDDEN", "Bridge access is restricted to the top frame")
        }

        val configured = handler.configuredOrigin()
        if (configured.isNullOrBlank() || !OriginPolicy.isSameOrigin(configured, sourceOrigin)) {
            return err(id, "ERR_ORIGIN_MISMATCH", "Origin $sourceOrigin is not authorized")
        }

        if (cmd in ADMIN_ONLY_COMMANDS) {
            return err(id, "ERR_ADMIN_LOCAL_ONLY", "Command $cmd is only available in Control Center")
        }
        if (cmd !in REMOTE_ALLOWED_COMMANDS) {
            return err(id, "ERR_UNKNOWN_COMMAND", "Unknown command: $cmd")
        }

        return try {
            val result: Any = when (cmd) {
                "print_pdf" -> handlePrintPdf(args.optJSONObject("req") ?: args)
                "print_text" -> handlePrintText(args.optJSONObject("req") ?: args)
                "print_image" -> handlePrintImage(args.optJSONObject("req") ?: args)
                "print_status" -> {
                    val jobId = args.optString("jobId", args.optString("job_id", ""))
                    handler.printStatus(jobId) ?: "not_found"
                }
                "job_detail" -> {
                    val jobId = args.optString("jobId", args.optString("job_id", ""))
                    handler.jobDetail(jobId)?.toJson() ?: JSONObject.NULL
                }
                "app_overview" -> handler.appOverview(args.optBoolean("force", false)).toJson()
                "list_printers" -> JSONArray(handler.listPrinters(args.optBoolean("force", false)).map { it.toJson() })
                "printer_forms" -> JSONArray(handler.printerForms(args.optString("printer", "")))
                "printer_capabilities" -> handler.printerCapabilities(args.optString("printer", ""))?.toJson() ?: JSONObject.NULL
                "open_control_center" -> { handler.openControlCenter(); true }
                "open_settings" -> { handler.openSettings(); true }
                "tab_open" -> {
                    val url = args.optString("url", "")
                    if (!OriginPolicy.isSameOrigin(configured, url)) {
                        return err(id, "ERR_OFF_ORIGIN_TAB", "Refused to open off-origin tab: $url")
                    }
                    handler.openTab(url)
                    true
                }
                "report_environment" -> {
                    handler.reportEnvironment(args.optString("report", ""))
                    true
                }
                else -> return err(id, "ERR_UNKNOWN_COMMAND", "Unknown command: $cmd")
            }
            JSONObject().apply {
                put("id", id)
                put("ok", true)
                put("result", result)
            }
        } catch (e: Exception) {
            err(id, "ERR_EXECUTION", e.message ?: e.javaClass.simpleName)
        }
    }

    private fun extractPrinter(req: JSONObject): String =
        req.optString("printer", "").ifBlank {
            req.optString("printerPattern", req.optString("printer_pattern", ""))
        }

    private fun handlePrintPdf(req: JSONObject): JSONObject {
        val printer = extractPrinter(req)
        val bytes = extractBytes(req.opt("bytes"), PayloadLimits.MAX_PDF_BYTES, "PDF_SIZE_INVALID")
        require(bytes.isNotEmpty() && bytes.size <= PayloadLimits.MAX_PDF_BYTES) {
            "PDF_SIZE_INVALID"
        }
        val copies = PayloadLimits.validateCopies(req.optInt("copies", 1))
        val idempotencyKey = req.optString("idempotency_key", req.optString("idempotencyKey", "")).ifBlank { null }
        return handler.enqueuePdf(printer, bytes, copies, idempotencyKey, req).toJson()
    }

    private fun handlePrintText(req: JSONObject): JSONObject {
        val printer = extractPrinter(req)
        val text = req.optString("text", req.optString("content", ""))
        val textBytes = text.toByteArray(Charsets.UTF_8)
        require(textBytes.isNotEmpty() && textBytes.size <= PayloadLimits.MAX_TEXT_BYTES) {
            "TEXT_SIZE_INVALID"
        }
        val copies = PayloadLimits.validateCopies(req.optInt("copies", 1))
        val idempotencyKey = req.optString("idempotency_key", req.optString("idempotencyKey", "")).ifBlank { null }
        return handler.enqueueText(printer, text, copies, idempotencyKey, req).toJson()
    }

    private fun handlePrintImage(req: JSONObject): JSONObject {
        val printer = extractPrinter(req)
        val bytes = extractBytes(req.opt("bytes"), PayloadLimits.MAX_IMAGE_BYTES, "IMAGE_SIZE_INVALID")
        require(bytes.isNotEmpty() && bytes.size <= PayloadLimits.MAX_IMAGE_BYTES) {
            "IMAGE_SIZE_INVALID"
        }
        val copies = PayloadLimits.validateCopies(req.optInt("copies", 1))
        val idempotencyKey = req.optString("idempotency_key", req.optString("idempotencyKey", "")).ifBlank { null }
        return handler.enqueueImage(printer, bytes, copies, idempotencyKey, req).toJson()
    }

    private fun extractBytes(raw: Any?, maxBytes: Int, errCode: String): ByteArray = when (raw) {
        is String -> {
            // Reject oversized Base64 before decoding to avoid OOM on low-RAM POS tablets.
            val maxEncodedLen = ((maxBytes.toLong() * 4L) / 3L) + 64L
            require(raw.length.toLong() <= maxEncodedLen) { errCode }
            decodeBase64(raw)
        }
        is JSONArray -> {
            require(raw.length() <= maxBytes) { errCode }
            ByteArray(raw.length()) { i -> (raw.optInt(i, 0) and 0xFF).toByte() }
        }
        else -> ByteArray(0)
    }

    private fun err(id: String, code: String, message: String): JSONObject = JSONObject().apply {
        put("id", id)
        put("ok", false)
        put("code", code)
        put("error", message)
    }

    companion object {
        val REMOTE_ALLOWED_COMMANDS: Set<String> = setOf(
            "print_pdf",
            "print_text",
            "print_image",
            "print_status",
            "job_detail",
            "app_overview",
            "list_printers",
            "printer_forms",
            "printer_capabilities",
            "open_control_center",
            "open_settings",
            "tab_open",
            "report_environment",
        )

        val ADMIN_ONLY_COMMANDS: Set<String> = setOf(
            "run_test_print",
            "retry_job_cmd",
            "cancel_job_cmd",
            "clear_finished_cmd",
            "export_diagnostics",
            "save_origin_cmd",
            "get_printer_settings",
            "set_printer_settings",
            "clear_web_data_cmd",
            "open_odoo_origin",
            "probe_origin",
        )
    }
}

/**
 * Android WebViewCompat.WebMessageListener binder executing bridge calls off the main thread.
 */
class AndroidWebBridgeListener(
    private val dispatcher: BridgeDispatcher,
    private val scope: CoroutineScope,
) : WebViewCompat.WebMessageListener {

    override fun onPostMessage(
        view: WebView,
        message: WebMessageCompat,
        sourceOrigin: android.net.Uri,
        isMainFrame: Boolean,
        replyProxy: JavaScriptReplyProxy,
    ) {
        val raw = message.data ?: return
        scope.launch(Dispatchers.IO) {
            val response = dispatcher.dispatch(
                sourceOrigin = sourceOrigin.toString(),
                isMainFrame = isMainFrame,
                rawMessage = raw,
            )
            launch(Dispatchers.Main) {
                runCatching { replyProxy.postMessage(response.toString()) }
            }
        }
    }
}
