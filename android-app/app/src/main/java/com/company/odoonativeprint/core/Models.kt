package com.company.odoonativeprint.core

import org.json.JSONArray
import org.json.JSONObject

enum class JobState(val wireName: String) {
    QUEUED("queued"),
    RENDERING("rendering"),
    SPOOLING("spooling"),
    SUBMITTED("submitted"),
    IN_PROGRESS("in_progress"),
    COMPLETED("done"),
    FAILED("failed"),
    CANCELLED("canceled");

    val isActive: Boolean
        get() = this == QUEUED || this == RENDERING || this == SPOOLING || this == SUBMITTED || this == IN_PROGRESS

    val isTerminal: Boolean
        get() = this == COMPLETED || this == FAILED || this == CANCELLED

    companion object {
        fun fromWire(value: String): JobState = when (value.lowercase()) {
            "completed", "done" -> COMPLETED
            "cancelled", "canceled" -> CANCELLED
            else -> entries.firstOrNull { it.wireName.equals(value, ignoreCase = true) } ?: FAILED
        }
    }
}

enum class PrinterTransportType(val wireName: String) {
    TCP_9100("tcp9100"),
    IPP("ipp"),
    BLUETOOTH_SPP("bluetooth_spp"),
    USB_OTG("usb_otg"),
    ANDROID_SYSTEM("android_system");

    companion object {
        fun fromWire(value: String): PrinterTransportType =
            entries.firstOrNull { it.wireName.equals(value, ignoreCase = true) } ?: TCP_9100
    }
}

enum class PrinterProtocol(val wireName: String) {
    AUTO("auto"),
    PDF("pdf"),
    ESCPOS("escpos"),
    ZPL("zpl"),
    TSPL("tspl"),
    EPL("epl"),
    CPCL("cpcl");

    companion object {
        fun fromWire(value: String): PrinterProtocol =
            entries.firstOrNull { it.wireName.equals(value, ignoreCase = true) } ?: AUTO
    }
}

data class PrinterInfo(
    val name: String,
    val displayName: String = name,
    val transport: PrinterTransportType,
    val address: String,
    val protocol: PrinterProtocol = PrinterProtocol.AUTO,
    val online: Boolean = true,
    val status: String = "ready",
    val isDefault: Boolean = false,
    val dpi: Int = 203,
    val paperWidthMm: Int = 80,
) {
    fun toJson(): JSONObject = JSONObject().apply {
        put("name", name)
        put("display_name", displayName)
        put("transport", transport.wireName)
        put("address", address)
        put("protocol", protocol.wireName)
        put("online", online)
        put("status", status)
        put("is_default", isDefault)
        put("dpi", dpi)
        put("paper_width_mm", paperWidthMm)
    }
}

data class PrinterCapabilities(
    val printer: String,
    val transport: PrinterTransportType,
    val protocol: PrinterProtocol,
    val dpi: Int,
    val paperWidthMm: Int,
    val forms: List<String>,
    val supportsColor: Boolean = false,
    val supportsDuplex: Boolean = false,
    val supportsRaw: Boolean = true,
    val supportsPdf: Boolean = true,
    val supportsCut: Boolean = true,
    val supportsDrawer: Boolean = true,
    val capsEpoch: Long = 0L,
) {
    fun toJson(): JSONObject = JSONObject().apply {
        put("printer", printer)
        put("transport", transport.wireName)
        put("protocol", protocol.wireName)
        put("dpi", dpi)
        put("paper_width_mm", paperWidthMm)
        put("forms", JSONArray(forms.map { formToJson(it) }))
        put("bins", JSONArray())
        put("color", supportsColor)
        put("duplex", supportsDuplex)
        put("supports_color", supportsColor)
        put("supports_duplex", supportsDuplex)
        put("supports_raw", supportsRaw)
        put("supports_pdf", supportsPdf)
        put("supports_cut", supportsCut)
        put("supports_drawer", supportsDrawer)
        put("caps_epoch", capsEpoch)
    }

    private fun formToJson(label: String): JSONObject {
        val lower = label.lowercase()
        val roll = lower.contains("roll")
        val (w, h) = when {
            lower == "a4" -> 210 to 297
            lower == "letter" -> 216 to 279
            else -> {
                val nums = Regex("""\d+""").findAll(label).mapNotNull { it.value.toIntOrNull() }.toList()
                (nums.getOrNull(0) ?: paperWidthMm) to (nums.getOrNull(1) ?: 0)
            }
        }
        return JSONObject().apply {
            put("name", label)
            put("width_mm", w)
            put("height_mm", h)
            put("roll", roll)
        }
    }
}

data class PrinterSettings(
    val protocol: PrinterProtocol = PrinterProtocol.AUTO,
    val paperWidthMm: Int = 80,
    val labelHeightMm: Int = 0,
    val dpi: Int = 203,
    val cutPaper: Boolean = true,
    val openCashDrawer: Boolean = false,
    val dithering: Boolean = false,
    val offsetXMm: Float = 0f,
    val offsetYMm: Float = 0f,
    val bluetoothPacingMs: Int = 0,
) {
    fun toJson(): JSONObject = JSONObject().apply {
        put("protocol", protocol.wireName)
        put("paper_width_mm", paperWidthMm)
        put("label_height_mm", labelHeightMm)
        put("dpi", dpi)
        put("cut_paper", cutPaper)
        put("open_cash_drawer", openCashDrawer)
        put("dithering", dithering)
        put("offset_x_mm", offsetXMm.toDouble())
        put("offset_y_mm", offsetYMm.toDouble())
        put("bluetooth_pacing_ms", bluetoothPacingMs)
    }

    companion object {
        fun fromJson(obj: JSONObject?): PrinterSettings {
            if (obj == null) return PrinterSettings()
            return PrinterSettings(
                protocol = PrinterProtocol.fromWire(obj.optString("protocol", "auto")),
                paperWidthMm = obj.optInt("paper_width_mm", 80).coerceIn(32, 297),
                labelHeightMm = obj.optInt("label_height_mm", 0).coerceIn(0, 600),
                dpi = obj.optInt("dpi", 203).coerceIn(150, 600),
                cutPaper = obj.optBoolean("cut_paper", true),
                openCashDrawer = obj.optBoolean("open_cash_drawer", false),
                dithering = obj.optBoolean("dithering", false),
                offsetXMm = obj.optDouble("offset_x_mm", 0.0).toFloat(),
                offsetYMm = obj.optDouble("offset_y_mm", 0.0).toFloat(),
                bluetoothPacingMs = obj.optInt("bluetooth_pacing_ms", 0).coerceIn(0, 50),
            )
        }
    }
}

data class ManualPrinterEntry(
    val name: String,
    val transport: PrinterTransportType,
    val address: String,
    val protocol: PrinterProtocol = PrinterProtocol.ESCPOS,
    val paperWidthMm: Int = 80,
    val dpi: Int = 203,
) {
    fun toJson(): JSONObject = JSONObject().apply {
        put("name", name)
        put("transport", transport.wireName)
        put("address", address)
        put("protocol", protocol.wireName)
        put("paper_width_mm", paperWidthMm)
        put("dpi", dpi)
    }

    companion object {
        fun fromJson(obj: JSONObject): ManualPrinterEntry = ManualPrinterEntry(
            name = obj.getString("name"),
            transport = PrinterTransportType.fromWire(obj.optString("transport", "tcp9100")),
            address = obj.getString("address"),
            protocol = PrinterProtocol.fromWire(obj.optString("protocol", "escpos")),
            paperWidthMm = obj.optInt("paper_width_mm", 80),
            dpi = obj.optInt("dpi", 203),
        )
    }
}

data class PrintJobRecord(
    val id: String,
    val idempotencyKey: String?,
    val printerName: String,
    val sourceKind: String,
    val state: JobState,
    val attempts: Int = 0,
    val maxAttempts: Int = 3,
    val pages: Int? = null,
    val copies: Int = 1,
    val optionsJson: String = "{}",
    val payloadPath: String,
    val errorCode: String? = null,
    val errorMessage: String? = null,
    val createdAtMs: Long,
    val updatedAtMs: Long,
    val nextRetryAtMs: Long = 0L,
) {
    fun toJson(): JSONObject = JSONObject().apply {
        put("id", id)
        put("job_id", id)
        if (idempotencyKey != null) put("idempotency_key", idempotencyKey)
        put("printer", printerName)
        put("source_kind", sourceKind)
        put("state", state.wireName)
        put("status", state.wireName)
        put("attempts", attempts)
        put("max_attempts", maxAttempts)
        if (pages != null) put("pages", pages)
        put("copies", copies)
        if (errorCode != null) put("error_code", errorCode)
        if (errorMessage != null) put("error_message", errorMessage)
        put("created_at_ms", createdAtMs)
        put("updated_at_ms", updatedAtMs)
    }
}

data class QueueSummary(
    val active: Int = 0,
    val queued: Int = 0,
    val failed: Int = 0,
    val completed: Int = 0,
) {
    fun toJson(): JSONObject = JSONObject().apply {
        put("active", active)
        put("queued", queued)
        put("failed", failed)
        put("completed", completed)
    }
}

data class AppOverview(
    val version: String,
    val platform: String = "android",
    val configuredOrigin: String?,
    val printers: List<PrinterInfo>,
    val queue: QueueSummary,
    val capsEpoch: Long,
    val defaultPrinter: String? = null,
) {
    fun toJson(): JSONObject = JSONObject().apply {
        put("version", version)
        put("platform", platform)
        put("origin", configuredOrigin ?: JSONObject.NULL)
        put("printers", JSONArray(printers.map { it.toJson() }))
        put("defaultPrinter", defaultPrinter ?: JSONObject.NULL)
        put("default_printer", defaultPrinter ?: JSONObject.NULL)
        put("queue", queue.toJson())
        put("caps_epoch", capsEpoch)
    }
}
