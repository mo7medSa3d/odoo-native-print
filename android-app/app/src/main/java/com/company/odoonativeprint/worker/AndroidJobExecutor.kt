package com.company.odoonativeprint.worker

import android.app.Activity
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import com.company.odoonativeprint.core.AppConfigStore
import com.company.odoonativeprint.core.PayloadLimits
import com.company.odoonativeprint.core.PrintJobRecord
import com.company.odoonativeprint.core.PrinterInfo
import com.company.odoonativeprint.core.PrinterProtocol
import com.company.odoonativeprint.core.PrinterSettings
import com.company.odoonativeprint.core.PrinterTransportType
import com.company.odoonativeprint.protocol.CpclEncoder
import com.company.odoonativeprint.protocol.EplEncoder
import com.company.odoonativeprint.protocol.EscPosEncoder
import com.company.odoonativeprint.protocol.TsplEncoder
import com.company.odoonativeprint.protocol.ZplEncoder
import com.company.odoonativeprint.render.BitmapToPdfWriter
import com.company.odoonativeprint.render.CalibrationPages
import com.company.odoonativeprint.render.MonoBitmap
import com.company.odoonativeprint.render.PdfPageRenderer
import com.company.odoonativeprint.render.TextReceiptRenderer
import com.company.odoonativeprint.transport.AndroidPrintManagerAdapter
import com.company.odoonativeprint.transport.BluetoothSppTransport
import com.company.odoonativeprint.transport.IppTransport
import com.company.odoonativeprint.transport.PrinterDiscoveryService
import com.company.odoonativeprint.transport.Tcp9100Transport
import com.company.odoonativeprint.transport.TransportResult
import com.company.odoonativeprint.transport.UsbOtgTransport
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.io.File
import kotlin.math.roundToInt

/**
 * Bridges queued PrintJobRecords through rendering -> protocol encoding -> physical transport.
 */
class AndroidJobExecutor(
    private val context: Context,
    private val configStore: AppConfigStore,
    private val discovery: PrinterDiscoveryService,
    private val tcp9100: Tcp9100Transport,
    private val ipp: IppTransport,
    private val bluetoothSpp: BluetoothSppTransport,
    private val usbOtg: UsbOtgTransport,
    private val activityProvider: () -> Activity? = { null },
) : JobExecutor {

    override suspend fun execute(job: PrintJobRecord): TransportResult {
        val printer = discovery.findPrinter(job.printerName)
            ?: return TransportResult.Failure(
                "PRINTER_NOT_FOUND",
                "Printer '${job.printerName}' not found",
                retryable = false,
            )
        val settings = configStore.getPrinterSettings(printer.name)
        val payloadFile = File(job.payloadPath)
        if (!payloadFile.exists()) {
            return TransportResult.Failure("PAYLOAD_MISSING", "Spool payload file is missing", retryable = false)
        }

        // Match standard 203-dpi thermal head widths; otherwise convert exact paperWidthMm to whole bytes.
        val widthDots = when {
            settings.dpi == 203 && settings.paperWidthMm in 54..62 -> 384
            settings.dpi == 203 && settings.paperWidthMm in 72..84 -> 576
            settings.dpi == 203 && settings.paperWidthMm in 104..120 -> 832
            else -> ((settings.paperWidthMm * settings.dpi / 25.4 / 8.0).toInt() * 8)
                .coerceIn(192, 1664)
        }

        // Android System Print accepts PDFs only. The POS sends receipts as
        // images, so convert the payload to a PDF here instead of rejecting it.
        // The bytes are snapshotted by the adapter: onWrite runs after the user
        // picks a destination, when the spool file may already be deleted.
        if (printer.transport == PrinterTransportType.ANDROID_SYSTEM) {
            val pdfBytes = try {
                when (job.sourceKind) {
                    "pdf" -> {
                        val bytes = payloadFile.readBytes()
                        require(bytes.isNotEmpty() && bytes.size <= PayloadLimits.MAX_PDF_BYTES) { "PDF_SIZE_INVALID" }
                        bytes
                    }
                    else -> {
                        val pages = renderPayload(job, printer, payloadFile, settings, widthDots)
                        try {
                            BitmapToPdfWriter.encode(pages, settings.paperWidthMm.toFloat())
                        } finally {
                            pages.forEach { it.recycle() }
                        }
                    }
                }
            } catch (e: Exception) {
                return TransportResult.Failure(
                    "SYSTEM_PRINT_PDF_FAILED",
                    e.message ?: "Could not prepare a PDF for Android System Print",
                    retryable = false,
                )
            }
            val activity = activityProvider()
                ?: return TransportResult.Failure(
                    "NO_ACTIVE_ACTIVITY",
                    "Open Yaseir Print to use Android System Print",
                    retryable = true,
                )
            return withContext(Dispatchers.Main) {
                AndroidPrintManagerAdapter.printPdfBytes(
                    activity = activity,
                    pdfBytes = pdfBytes,
                    documentName = "Yaseir-${job.id.take(8)}.pdf",
                    jobName = "Yaseir Print #${job.id.take(8)}",
                )
            }
        }

        val protocol = resolveProtocol(printer, settings)

        // Pass explicit raw label commands (e.g. Odoo ZPL qweb-text) straight to the printer
        // instead of rasterizing literal ^XA...^XZ source code through TextReceiptRenderer.
        val jobOpts = runCatching { JSONObject(job.optionsJson) }.getOrDefault(JSONObject())
        val requestedMode = jobOpts.optString("mode", "").lowercase()
        if (shouldPassRawLabelBytes(job.sourceKind, settings.protocol, protocol, requestedMode)) {
            val single = payloadFile.readBytes()
            val out = ByteArrayOutputStream(single.size * job.copies.coerceAtLeast(1))
            repeat(job.copies.coerceAtLeast(1)) { out.write(single) }
            return sendRawPayload(printer, out.toByteArray(), settings)
        }

        // IPP printers expect PDF (never raw ESC/POS unless the user explicitly forced a raw protocol).
        if (printer.transport == PrinterTransportType.IPP &&
            (job.sourceKind == "pdf" || protocol == PrinterProtocol.PDF)
        ) {
            val duplex = jobOpts.optBoolean("duplex", false)
            val sides = if (duplex) "two-sided-long-edge" else "one-sided"
            if (job.sourceKind == "pdf") {
                return ipp.sendDocument(
                    printer = printer,
                    documentFile = payloadFile,
                    mimeType = "application/pdf",
                    jobName = "Yaseir-${job.id.take(8)}",
                    copies = job.copies,
                    sides = sides,
                )
            }
            val pdfBytes = try {
                val pages = renderPayload(job, printer, payloadFile, settings, widthDots)
                try {
                    BitmapToPdfWriter.encode(pages, settings.paperWidthMm.toFloat())
                } finally {
                    pages.forEach { it.recycle() }
                }
            } catch (e: Exception) {
                return TransportResult.Failure(
                    "RENDER_FAILED",
                    e.message ?: "Failed to render PDF for IPP printer",
                    retryable = false,
                )
            }
            return ipp.sendDocument(
                printer = printer,
                documentBytes = pdfBytes,
                mimeType = "application/pdf",
                jobName = "Yaseir-${job.id.take(8)}",
                copies = job.copies,
                sides = sides,
            )
        }

        // Stream one rendered page at a time so multi-page PDFs never hold all bitmaps in RAM at once.
        val encodedPages = mutableListOf<ByteArray>()
        try {
            renderEachPage(job, printer, payloadFile, settings, widthDots) { bmp ->
                val mono = MonoBitmap.fromAndroidBitmap(bmp, dither = settings.dithering)
                encodedPages += when (protocol) {
                    PrinterProtocol.ZPL -> ZplEncoder.encode(mono, copies = 1)
                    PrinterProtocol.TSPL -> TsplEncoder.encode(
                        mono,
                        widthMm = settings.paperWidthMm,
                        heightMm = settings.labelHeightMm.takeIf { it > 0 }
                            ?: (mono.height * 25.4f / settings.dpi).roundToInt().coerceIn(10, 600),
                        copies = 1,
                    )
                    PrinterProtocol.EPL -> EplEncoder.encode(mono, copies = 1)
                    PrinterProtocol.CPCL -> CpclEncoder.encode(mono, copies = 1, dpi = settings.dpi)
                    else -> EscPosEncoder.encodeRaster(
                        mono = mono,
                        cut = false,
                        openDrawer = false,
                    )
                }
            }
        } catch (e: Exception) {
            return TransportResult.Failure("RENDER_FAILED", e.message ?: "Failed to render payload", retryable = false)
        }

        val isEscPos = protocol !in setOf(PrinterProtocol.ZPL, PrinterProtocol.TSPL, PrinterProtocol.EPL, PrinterProtocol.CPCL)
        val encodedStream = ByteArrayOutputStream()
        val copies = job.copies.coerceAtLeast(1)
        repeat(copies) {
            for (pageBytes in encodedPages) {
                encodedStream.write(pageBytes)
            }
            if (isEscPos && settings.cutPaper && encodedPages.isNotEmpty()) {
                encodedStream.write(EscPosEncoder.CUT_COMMAND)
            }
        }
        if (isEscPos && settings.openCashDrawer && encodedPages.isNotEmpty()) {
            encodedStream.write(EscPosEncoder.OPEN_DRAWER_COMMAND)
        }

        return sendRawPayload(printer, encodedStream.toByteArray(), settings)
    }

    private suspend fun sendRawPayload(
        printer: PrinterInfo,
        rawBytes: ByteArray,
        settings: PrinterSettings,
    ): TransportResult = when (printer.transport) {
        PrinterTransportType.TCP_9100 -> tcp9100.sendRaw(printer, rawBytes, settings)
        PrinterTransportType.BLUETOOTH_SPP -> bluetoothSpp.sendRaw(printer, rawBytes, settings)
        PrinterTransportType.USB_OTG -> usbOtg.sendRaw(printer, rawBytes, settings)
        PrinterTransportType.IPP -> ipp.sendDocument(printer, rawBytes, mimeType = "application/octet-stream", copies = 1)
        PrinterTransportType.ANDROID_SYSTEM -> TransportResult.Failure("UNSUPPORTED_TRANSPORT", "Unsupported transport", retryable = false)
    }

    private suspend fun renderEachPage(
        job: PrintJobRecord,
        printer: PrinterInfo,
        payloadFile: File,
        settings: PrinterSettings,
        widthDots: Int,
        onPage: (Bitmap) -> Unit,
    ) {
        when (job.sourceKind) {
            "pdf" -> PdfPageRenderer.renderEachPage(
                pdfFile = payloadFile,
                targetWidthDots = widthDots,
                offsetXDots = (settings.offsetXMm * settings.dpi / 25.4f),
                offsetYDots = (settings.offsetYMm * settings.dpi / 25.4f),
            ) { _, bmp -> onPage(bmp) }
            "image" -> {
                val bmp = decodeValidatedImage(payloadFile, widthDots)
                try {
                    onPage(bmp)
                } finally {
                    if (!bmp.isRecycled) bmp.recycle()
                }
            }
            "text" -> {
                val bmp = TextReceiptRenderer.render(
                    context,
                    payloadFile.readText(Charsets.UTF_8),
                    widthDots = widthDots,
                )
                try {
                    onPage(bmp)
                } finally {
                    if (!bmp.isRecycled) bmp.recycle()
                }
            }
            "test" -> {
                val bmp = CalibrationPages.thermalTestReceipt(context, printer.displayName, settings.paperWidthMm, settings.dpi)
                try {
                    onPage(bmp)
                } finally {
                    if (!bmp.isRecycled) bmp.recycle()
                }
            }
            else -> throw IllegalArgumentException("Unsupported kind: ${job.sourceKind}")
        }
    }

    private suspend fun renderPayload(
        job: PrintJobRecord,
        printer: PrinterInfo,
        payloadFile: File,
        settings: PrinterSettings,
        widthDots: Int,
    ): List<Bitmap> = when (job.sourceKind) {
        "pdf" -> PdfPageRenderer.renderAllPages(
            pdfFile = payloadFile,
            targetWidthDots = widthDots,
            offsetXDots = (settings.offsetXMm * settings.dpi / 25.4f),
            offsetYDots = (settings.offsetYMm * settings.dpi / 25.4f),
        )
        "image" -> listOf(decodeValidatedImage(payloadFile, widthDots))
        "text" -> listOf(
            TextReceiptRenderer.render(
                context,
                payloadFile.readText(Charsets.UTF_8),
                widthDots = widthDots,
            )
        )
        "test" -> listOf(
            CalibrationPages.thermalTestReceipt(context, printer.displayName, settings.paperWidthMm, settings.dpi)
        )
        else -> throw IllegalArgumentException("Unsupported kind: ${job.sourceKind}")
    }

    private fun resolveProtocol(printer: PrinterInfo, settings: PrinterSettings): PrinterProtocol =
        if (settings.protocol != PrinterProtocol.AUTO) {
            settings.protocol
        } else if (printer.protocol != PrinterProtocol.AUTO) {
            printer.protocol
        } else {
            PrinterProtocol.ESCPOS
        }

    private fun decodeValidatedImage(file: File, targetWidthDots: Int): Bitmap {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeFile(file.absolutePath, bounds)
        require(bounds.outWidth > 0 && bounds.outHeight > 0) { "IMAGE_DECODE_FAILED" }
        require(bounds.outWidth <= PayloadLimits.MAX_IMAGE_SIDE && bounds.outHeight <= PayloadLimits.MAX_IMAGE_SIDE) {
            "IMAGE_DIMENSION_EXCEEDED"
        }
        require(bounds.outWidth.toLong() * bounds.outHeight.toLong() <= PayloadLimits.MAX_IMAGE_PIXELS) {
            "IMAGE_PIXEL_LIMIT_EXCEEDED"
        }
        var sampleSize = 1
        while (bounds.outWidth / (sampleSize * 2) >= targetWidthDots) {
            sampleSize *= 2
        }
        val decodeOpts = BitmapFactory.Options().apply { inSampleSize = sampleSize }
        val decoded = BitmapFactory.decodeFile(file.absolutePath, decodeOpts)
            ?: throw IllegalArgumentException("IMAGE_DECODE_FAILED")
        if (decoded.width == targetWidthDots) return decoded
        val targetH = ((decoded.height.toLong() * targetWidthDots) / decoded.width).toInt().coerceAtLeast(1)
        val scaled = Bitmap.createScaledBitmap(decoded, targetWidthDots, targetH, true)
        if (scaled !== decoded) decoded.recycle()
        return scaled
    }

    companion object {
        private val RAW_LABEL_PROTOCOLS = setOf(
            PrinterProtocol.ZPL,
            PrinterProtocol.TSPL,
            PrinterProtocol.EPL,
            PrinterProtocol.CPCL,
        )
        private val RAW_LABEL_MODES = setOf("zpl", "tspl", "epl", "cpcl")

        internal fun shouldPassRawLabelBytes(
            sourceKind: String,
            settingsProtocol: PrinterProtocol,
            resolvedProtocol: PrinterProtocol,
            requestedMode: String,
        ): Boolean {
            if (sourceKind != "text") return false
            val mode = requestedMode.trim().lowercase()
            if (resolvedProtocol in RAW_LABEL_PROTOCOLS && mode == resolvedProtocol.wireName) return true
            return settingsProtocol == PrinterProtocol.AUTO && mode in RAW_LABEL_MODES
        }
    }
}
