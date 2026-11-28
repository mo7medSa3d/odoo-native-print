package com.company.odoonativeprint.render

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Matrix
import android.graphics.pdf.PdfRenderer
import android.os.ParcelFileDescriptor
import com.company.odoonativeprint.core.PayloadLimits
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.io.File
import kotlin.math.roundToInt

/**
 * Thread-safe wrapper around Android's built-in PdfRenderer (zero external PDFium .so bloat).
 */
object PdfPageRenderer {
    private const val MAX_PAGE_HEIGHT_PX = 16_384
    private val renderMutex = Mutex()

    suspend fun renderEachPage(
        pdfFile: File,
        targetWidthDots: Int,
        offsetXDots: Float = 0f,
        offsetYDots: Float = 0f,
        pageRange: IntRange? = null,
        onPage: (Int, Bitmap) -> Unit,
    ) = renderMutex.withLock {
        val pfd = ParcelFileDescriptor.open(pdfFile, ParcelFileDescriptor.MODE_READ_ONLY)
        pfd.use { fd ->
            PdfRenderer(fd).use { renderer ->
                val count = renderer.pageCount
                require(count > 0) { "PDF_EMPTY" }
                require(count <= PayloadLimits.MAX_PDF_PAGES) {
                    "PDF_PAGE_LIMIT_EXCEEDED: $count > ${PayloadLimits.MAX_PDF_PAGES}"
                }

                val start = (pageRange?.first ?: 0).coerceIn(0, count - 1)
                val end = (pageRange?.last ?: (count - 1)).coerceIn(start, count - 1)

                for (index in start..end) {
                    renderer.openPage(index).use { page ->
                        val scale = targetWidthDots.toFloat() / page.width.coerceAtLeast(1).toFloat()
                        val outW = targetWidthDots.coerceAtLeast(1)
                        val outH = (page.height * scale).roundToInt().coerceIn(1, MAX_PAGE_HEIGHT_PX)
                        val bmp = Bitmap.createBitmap(outW, outH, Bitmap.Config.ARGB_8888)
                        try {
                            val canvas = Canvas(bmp)
                            canvas.drawColor(Color.WHITE)
                            val matrix = Matrix().apply {
                                postScale(scale, scale)
                                if (offsetXDots != 0f || offsetYDots != 0f) {
                                    postTranslate(offsetXDots, offsetYDots)
                                }
                            }
                            page.render(bmp, null, matrix, PdfRenderer.Page.RENDER_MODE_FOR_PRINT)
                            onPage(index, bmp)
                        } finally {
                            if (!bmp.isRecycled) bmp.recycle()
                        }
                    }
                }
            }
        }
    }

    suspend fun renderAllPages(
        pdfFile: File,
        targetWidthDots: Int,
        offsetXDots: Float = 0f,
        offsetYDots: Float = 0f,
        pageRange: IntRange? = null,
    ): List<Bitmap> = renderMutex.withLock {
        val pfd = ParcelFileDescriptor.open(pdfFile, ParcelFileDescriptor.MODE_READ_ONLY)
        pfd.use { fd ->
            PdfRenderer(fd).use { renderer ->
                val count = renderer.pageCount
                require(count > 0) { "PDF_EMPTY" }
                require(count <= PayloadLimits.MAX_PDF_PAGES) {
                    "PDF_PAGE_LIMIT_EXCEEDED: $count > ${PayloadLimits.MAX_PDF_PAGES}"
                }

                val start = (pageRange?.first ?: 0).coerceIn(0, count - 1)
                val end = (pageRange?.last ?: (count - 1)).coerceIn(start, count - 1)
                val pages = ArrayList<Bitmap>(end - start + 1)
                try {
                    for (index in start..end) {
                        renderer.openPage(index).use { page ->
                            val scale = targetWidthDots.toFloat() / page.width.coerceAtLeast(1).toFloat()
                            val outW = targetWidthDots.coerceAtLeast(1)
                            val outH = (page.height * scale).roundToInt().coerceIn(1, MAX_PAGE_HEIGHT_PX)
                            val bmp = Bitmap.createBitmap(outW, outH, Bitmap.Config.ARGB_8888)
                            try {
                                val canvas = Canvas(bmp)
                                canvas.drawColor(Color.WHITE)
                                val matrix = Matrix().apply {
                                    postScale(scale, scale)
                                    if (offsetXDots != 0f || offsetYDots != 0f) {
                                        postTranslate(offsetXDots, offsetYDots)
                                    }
                                }
                                page.render(bmp, null, matrix, PdfRenderer.Page.RENDER_MODE_FOR_PRINT)
                                pages += bmp
                            } catch (t: Throwable) {
                                if (!bmp.isRecycled) bmp.recycle()
                                throw t
                            }
                        }
                    }
                    pages
                } catch (t: Throwable) {
                    pages.forEach { if (!it.isRecycled) it.recycle() }
                    throw t
                }
            }
        }
    }
}
