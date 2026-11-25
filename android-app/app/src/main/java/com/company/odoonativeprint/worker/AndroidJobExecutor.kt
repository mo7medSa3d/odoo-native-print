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

        // Render width is shared by raw protocols and by the PDF fallback.
        // Match standard 203-dpi thermal head widths (58mm->384, 80mm->576, 112mm->832),
        // rounded to whole raster bytes so GS v 0 never overflows the head.
        val widthDots = when {
            settings.dpi == 203 && settings.paperWidthMm in 54..62 -> 384
            settings.dpi == 203 && settings.paperWidthMm in 72..84 -> 576
            settings.dpi == 203 && settings.paperWidthMm in 104..120 -> 832
            else -> (((settings.paperWidthMm - 8).coerceAtLeast(32) * settings.dpi / 25.4 / 8.0).toInt() * 8)
                .coerceIn(256, 1664)
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

        // Fast path: IPP direct PDF submission
        if (printer.transport == PrinterTransportType.IPP && job.sourceKind == "pdf") {
            val opts = runCatching { JSONObject(job.optionsJson) }.getOrDefault(JSONObject())
            val duplex = opts.optBoolean("duplex", false)
            return ipp.sendDocument(
                printer = printer,
                documentBytes = payloadFile.readBytes(),
                mimeType = "application/pdf",
                jobName = "Yaseir-${job.id.take(8)}",
                copies = job.copies,
                sides = if (duplex) "two-sided-long-edge" else "one-sided",
            )
        }

        val bitmaps: List<Bitmap> = try {
            renderPayload(job, printer, payloadFile, settings, widthDots)
        } catch (e: Exception) {
            return TransportResult.Failure("RENDER_FAILED", e.message ?: "Failed to render payload", retryable = false)
        }

        val protocol = resolveProtocol(printer, settings)
        val encodedStream = ByteArrayOutputStream()
        try {
            val encodedPages = bitmaps.map { bmp ->
                val mono = MonoBitmap.fromAndroidBitmap(bmp, dither = settings.dithering)
                when (protocol) {
                    PrinterProtocol.ZPL -> ZplEncoder.encode(mono, copies = job.copies)
                    PrinterProtocol.TSPL -> TsplEncoder.encode(
                        mono,
                        widthMm = settings.paperWidthMm,
                        heightMm = settings.labelHeightMm.takeIf { it > 0 } ?: 100,
                        copies = job.copies,
                    )
                    PrinterProtocol.EPL -> EplEncoder.encode(mono, copies = job.copies)
                    PrinterProtocol.CPCL -> CpclEncoder.encode(mono, copies = job.copies)
                    else -> EscPosEncoder.encodeRaster(
                        mono = mono,
                        cut = settings.cutPaper,
                        openDrawer = settings.openCashDrawer,
                    )
                }
            }
            val copies = if (protocol == PrinterProtocol.ESCPOS) job.copies else 1
            repeat(copies) {
                for (pageBytes in encodedPages) {
                    encodedStream.write(pageBytes)
                }
            }
        } finally {
            bitmaps.forEach { it.recycle() }
        }

        val rawBytes = encodedStream.toByteArray()
        return when (printer.transport) {
            PrinterTransportType.TCP_9100 -> tcp9100.sendRaw(printer, rawBytes, settings)
            PrinterTransportType.BLUETOOTH_SPP -> bluetoothSpp.sendRaw(printer, rawBytes, settings)
            PrinterTransportType.USB_OTG -> usbOtg.sendRaw(printer, rawBytes, settings)
            PrinterTransportType.IPP -> ipp.sendDocument(printer, rawBytes, mimeType = "application/octet-stream", copies = 1)
            PrinterTransportType.ANDROID_SYSTEM -> TransportResult.Failure("UNSUPPORTED_TRANSPORT", "Unsupported transport", retryable = false)
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
        val decoded = BitmapFactory.decodeFile(file.absolutePath)
            ?: throw IllegalArgumentException("IMAGE_DECODE_FAILED")
        if (decoded.width == targetWidthDots) return decoded
        val targetH = ((decoded.height.toLong() * targetWidthDots) / decoded.width).toInt().coerceAtLeast(1)
        val scaled = Bitmap.createScaledBitmap(decoded, targetWidthDots, targetH, true)
        if (scaled !== decoded) decoded.recycle()
        return scaled
    }
}
