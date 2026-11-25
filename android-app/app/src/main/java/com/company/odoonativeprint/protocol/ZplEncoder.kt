package com.company.odoonativeprint.protocol

import com.company.odoonativeprint.render.MonoBitmap

/**
 * Pure-Kotlin Zebra ZPL II encoder matching crates/print-raw/src/zpl.rs.
 */
object ZplEncoder {
    fun encode(mono: MonoBitmap, copies: Int = 1): ByteArray {
        val totalBytes = mono.data.size
        val hex = mono.toHexUppercase()
        val zpl = buildString(totalBytes * 2 + 96) {
            append("^XA\n")
            append("^PW").append(mono.width).append('\n')
            append("^LL").append(mono.height).append('\n')
            append("^LH0,0\n")
            append("^FO0,0^GFA,")
                .append(totalBytes).append(',')
                .append(totalBytes).append(',')
                .append(mono.rowBytes).append(',')
                .append(hex)
                .append("^FS\n")
            if (copies > 1) {
                append("^PQ").append(copies.coerceIn(1, 99)).append('\n')
            }
            append("^XZ\n")
        }
        return zpl.toByteArray(Charsets.US_ASCII)
    }
}
