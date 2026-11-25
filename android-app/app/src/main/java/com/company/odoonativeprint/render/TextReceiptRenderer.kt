package com.company.odoonativeprint.render

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Typeface
import android.text.Layout
import android.text.StaticLayout
import android.text.TextPaint
import androidx.core.content.res.ResourcesCompat
import com.company.odoonativeprint.R

/**
 * Renders Arabic/RTL and mixed-script receipt text into a crisp monochrome-ready Bitmap
 * using Thmanyah Sans so thermal printers never garble Arabic glyphs regardless of codepage.
 */
object TextReceiptRenderer {

    fun render(
        context: Context,
        text: String,
        widthDots: Int = 576,
        textSizePx: Float = 26f,
        paddingPx: Int = 16,
    ): Bitmap {
        val typeface = ResourcesCompat.getFont(context, R.font.thmanyah_sans_medium) ?: Typeface.DEFAULT
        val paint = TextPaint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.BLACK
            textSize = textSizePx
            this.typeface = typeface
        }
        val contentWidth = (widthDots - paddingPx * 2).coerceAtLeast(120)
        val layout = StaticLayout.Builder
            .obtain(text, 0, text.length, paint, contentWidth)
            .setAlignment(Layout.Alignment.ALIGN_CENTER)
            .setLineSpacing(4f, 1.15f)
            .setIncludePad(true)
            .build()

        val height = (layout.height + paddingPx * 2).coerceAtLeast(64)
        val bmp = Bitmap.createBitmap(widthDots, height, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bmp)
        canvas.drawColor(Color.WHITE)
        canvas.save()
        canvas.translate(paddingPx.toFloat(), paddingPx.toFloat())
        layout.draw(canvas)
        canvas.restore()
        return bmp
    }
}
