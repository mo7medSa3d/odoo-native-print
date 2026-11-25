package com.company.odoonativeprint.protocol

import com.company.odoonativeprint.render.MonoBitmap
import java.io.ByteArrayOutputStream

/**
 * Pure-Kotlin TSC TSPL2 encoder matching crates/print-raw/src/tspl.rs.
 * Note: TSPL BITMAP uses 0 = black dot, 1 = white dot (inverted relative to MonoBitmap).
 */
object TsplEncoder {
    fun encode(mono: MonoBitmap, widthMm: Int, heightMm: Int, copies: Int = 1): ByteArray {
        val out = ByteArrayOutputStream(mono.data.size + 128)
        val header = buildString {
            append("SIZE ").append(widthMm).append(" mm, ").append(heightMm).append(" mm\r\n")
            append("GAP 2 mm, 0 mm\r\n")
            append("DIRECTION 1\r\n")
            append("CLS\r\n")
            append("BITMAP 0,0,")
                .append(mono.rowBytes).append(',')
                .append(mono.height).append(",0,")
        }
        out.write(header.toByteArray(Charsets.US_ASCII))
        out.write(mono.invertedCopy())
        out.write("\r\nPRINT ${copies.coerceIn(1, 99)},1\r\n".toByteArray(Charsets.US_ASCII))
        return out.toByteArray()
    }
}
