package com.company.odoonativeprint.render

import android.graphics.Bitmap
import android.graphics.Color

/**
 * 1-bit packed monochrome bitmap (MSB first, bit = 1 means black ink dot).
 * Pure JVM data structure so protocol encoders and tests run without Android runtime.
 */
data class MonoBitmap(
    val width: Int,
    val height: Int,
    val rowBytes: Int,
    val data: ByteArray,
) {
    init {
        require(width > 0 && height > 0) { "INVALID_MONO_DIMENSIONS" }
        require(rowBytes == (width + 7) / 8) { "INVALID_ROW_BYTES" }
        require(data.size == rowBytes * height) { "INVALID_DATA_SIZE" }
    }

    fun toHexUppercase(): String {
        val hexChars = "0123456789ABCDEF"
        val out = StringBuilder(data.size * 2)
        for (b in data) {
            val v = b.toInt() and 0xFF
            out.append(hexChars[v ushr 4])
            out.append(hexChars[v and 0x0F])
        }
        return out.toString()
    }

    fun invertedCopy(): ByteArray = ByteArray(data.size) { i -> (data[i].toInt() xor 0xFF).toByte() }

    companion object {
        /**
         * Converts grayscale luminance (0 = black, 255 = white) into 1-bit MonoBitmap.
         */
        fun fromLuminance(
            width: Int,
            height: Int,
            luminance: IntArray,
            dither: Boolean = true,
            threshold: Int = 160,
        ): MonoBitmap {
            require(luminance.size == width * height) { "LUMINANCE_SIZE_MISMATCH" }
            val work = FloatArray(luminance.size) { luminance[it].toFloat() }
            val rowBytes = (width + 7) / 8
            val packed = ByteArray(rowBytes * height)

            for (y in 0 until height) {
                val rowOff = y * width
                val byteRowOff = y * rowBytes
                for (x in 0 until width) {
                    val idx = rowOff + x
                    val oldVal = work[idx].coerceIn(0f, 255f)
                    val isBlack = oldVal < threshold
                    val newVal = if (isBlack) 0f else 255f
                    if (isBlack) {
                        packed[byteRowOff + (x ushr 3)] =
                            (packed[byteRowOff + (x ushr 3)].toInt() or (0x80 ushr (x and 7))).toByte()
                    }
                    if (dither) {
                        val err = oldVal - newVal
                        if (x + 1 < width) work[idx + 1] += err * (7f / 16f)
                        if (y + 1 < height) {
                            val nextRow = idx + width
                            if (x > 0) work[nextRow - 1] += err * (3f / 16f)
                            work[nextRow] += err * (5f / 16f)
                            if (x + 1 < width) work[nextRow + 1] += err * (1f / 16f)
                        }
                    }
                }
            }
            return MonoBitmap(width, height, rowBytes, packed)
        }

        fun fromAndroidBitmap(bitmap: Bitmap, dither: Boolean = true, threshold: Int = 160): MonoBitmap {
            val w = bitmap.width
            val h = bitmap.height
            val pixels = IntArray(w * h)
            bitmap.getPixels(pixels, 0, w, 0, 0, w, h)
            val lum = IntArray(w * h) { i ->
                val p = pixels[i]
                val a = Color.alpha(p)
                if (a < 32) {
                    255
                } else {
                    // Composite over white background so transparent PNGs print cleanly
                    val r = (Color.red(p) * a + 255 * (255 - a)) / 255
                    val g = (Color.green(p) * a + 255 * (255 - a)) / 255
                    val b = (Color.blue(p) * a + 255 * (255 - a)) / 255
                    (r * 299 + g * 587 + b * 114) / 1000
                }
            }
            return fromLuminance(w, h, lum, dither = dither, threshold = threshold)
        }
    }
}
