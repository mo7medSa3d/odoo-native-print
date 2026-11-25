package com.company.odoonativeprint.transport

import android.app.Activity
import android.content.Context
import android.os.Bundle
import android.os.CancellationSignal
import android.os.ParcelFileDescriptor
import android.print.PageRange
import android.print.PrintAttributes
import android.print.PrintDocumentAdapter
import android.print.PrintDocumentInfo
import android.print.PrintManager
import java.io.FileOutputStream

/**
 * Optional fallback that hands an in-memory PDF to Android's native PrintManager.
 *
 * The bytes are captured when print() is called, not when onWrite() is invoked.
 * Android calls onWrite only after the user picks a destination (possibly much
 * later), by which time the queue may have deleted the spool file.
 */
object AndroidPrintManagerAdapter {

    fun printPdfBytes(
        activity: Activity,
        pdfBytes: ByteArray,
        documentName: String,
        jobName: String = "Yaseir Print",
    ): TransportResult {
        val printManager = activity.getSystemService(Context.PRINT_SERVICE) as? PrintManager
            ?: return TransportResult.Failure("PRINT_MANAGER_UNAVAILABLE", "Android PrintManager unavailable", retryable = false)

        val name = documentName.ifBlank { "yaseir-print.pdf" }
        val snapshot = pdfBytes
        val adapter = object : PrintDocumentAdapter() {
            override fun onLayout(
                oldAttributes: PrintAttributes?,
                newAttributes: PrintAttributes?,
                cancellationSignal: CancellationSignal?,
                callback: LayoutResultCallback,
                extras: Bundle?,
            ) {
                if (cancellationSignal?.isCanceled == true) {
                    callback.onLayoutCancelled()
                    return
                }
                val info = PrintDocumentInfo.Builder(name)
                    .setContentType(PrintDocumentInfo.CONTENT_TYPE_DOCUMENT)
                    .setPageCount(PrintDocumentInfo.PAGE_COUNT_UNKNOWN)
                    .build()
                callback.onLayoutFinished(info, true)
            }

            override fun onWrite(
                pages: Array<out PageRange>?,
                destination: ParcelFileDescriptor,
                cancellationSignal: CancellationSignal?,
                callback: WriteResultCallback,
            ) {
                try {
                    FileOutputStream(destination.fileDescriptor).use { output ->
                        output.write(snapshot)
                    }
                    callback.onWriteFinished(arrayOf(PageRange.ALL_PAGES))
                } catch (e: Exception) {
                    callback.onWriteFailed(e.message)
                }
            }
        }
        printManager.print(jobName, adapter, null)
        return TransportResult.Ok
    }
}
