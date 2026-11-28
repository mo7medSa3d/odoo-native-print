package com.company.odoonativeprint.render

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import com.company.odoonativeprint.BuildConfig

/**
 * Generates thermal test receipts and alignment rulers matching desktop calibration pages.
 */
object CalibrationPages {

    fun thermalTestReceipt(context: Context, printerName: String, paperWidthMm: Int, dpi: Int): Bitmap {
        val widthDots = when {
            dpi == 203 && paperWidthMm in 54..62 -> 384
            dpi == 203 && paperWidthMm in 72..84 -> 576
            dpi == 203 && paperWidthMm in 104..120 -> 832
            else -> ((paperWidthMm * dpi / 25.4 / 8.0).toInt() * 8).coerceIn(192, 1664)
        }
        val text = buildString {
            appendLine("Itbaa — اختبار الطباعة")
            appendLine("════════════════════════════")
            appendLine("الطابعة: $printerName")
            appendLine("العرض: ${paperWidthMm}mm | الدقة: ${dpi} DPI")
            appendLine("الإصدار: Android v${BuildConfig.VERSION_NAME}")
            appendLine("الحالة: الاتصال والترميز العربي سليم ✓")
            appendLine("────────────────────────────")
            appendLine("1234567890 | أ ب ج د هـ و ز")
        }
        val textBmp = TextReceiptRenderer.render(context, text, widthDots = widthDots)
        val textHeight = textBmp.height
        // Draw a crisp alignment border + 10mm ruler ticks at bottom
        val rulerHeight = 48
        val out = Bitmap.createBitmap(widthDots, textHeight + rulerHeight, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(out)
        canvas.drawColor(Color.WHITE)
        canvas.drawBitmap(textBmp, 0f, 0f, null)
        textBmp.recycle()

        val paint = Paint().apply {
            color = Color.BLACK
            strokeWidth = 2f
            style = Paint.Style.STROKE
        }
        val top = textHeight.toFloat() + 6f
        canvas.drawLine(4f, top, (widthDots - 4).toFloat(), top, paint)
        val stepDots = (10.0 * dpi / 25.4).toFloat()
        var x = 4f
        while (x < widthDots - 4) {
            canvas.drawLine(x, top, x, top + 20f, paint)
            x += stepDots
        }
        return out
    }
}
