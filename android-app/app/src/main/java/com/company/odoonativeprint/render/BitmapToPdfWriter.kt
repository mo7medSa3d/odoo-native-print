package com.company.odoonativeprint.render

import android.graphics.Bitmap
import android.graphics.RectF
import android.graphics.pdf.PdfDocument
import java.io.ByteArrayOutputStream
import kotlin.math.roundToInt

/**
 * Wraps already-rendered receipt bitmaps in a PDF so Android System Print can
 * accept text/image payloads. The POS sends receipts as PNGs; the system print
 * dialog only understands PDF documents.
 */
object BitmapToPdfWriter {
    private const val MAX_PAGE_HEIGHT_POINTS = 20_000

    fun encode(bitmaps: List<Bitmap>, paperWidthMm: Float = 80f): ByteArray {
        require(bitmaps.isNotEmpty()) { "PDF_NO_PAGES" }
        val pageWidth = ((paperWidthMm * 72f) / 25.4f).roundToInt().coerceAtLeast(1)
        val document = PdfDocument()
        try {
            bitmaps.forEachIndexed { index, bitmap ->
                require(bitmap.width > 0 && bitmap.height > 0) { "PDF_INVALID_PAGE" }
                val pageHeight = (bitmap.height.toLong() * pageWidth / bitmap.width)
                    .coerceAtLeast(1)
                    .coerceAtMost(MAX_PAGE_HEIGHT_POINTS.toLong())
                    .toInt()
                val page = document.startPage(
                    PdfDocument.PageInfo.Builder(pageWidth, pageHeight, index + 1).create()
                )
                try {
                    page.canvas.drawBitmap(bitmap, null, RectF(0f, 0f, pageWidth.toFloat(), pageHeight.toFloat()), null)
                } finally {
                    document.finishPage(page)
                }
            }
            val output = ByteArrayOutputStream()
            document.writeTo(output)
            return output.toByteArray()
        } finally {
            document.close()
        }
    }
}
