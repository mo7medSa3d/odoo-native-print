package com.company.odoonativeprint.protocol

import com.company.odoonativeprint.render.MonoBitmap

/**
 * Pure-Kotlin CPCL encoder for mobile Bluetooth label/receipt printers.
 */
object CpclEncoder {
    fun encode(mono: MonoBitmap, copies: Int = 1, dpi: Int = 200): ByteArray {
        val hex = mono.toHexUppercase()
        val safeDpi = dpi.coerceIn(150, 600)
        val body = buildString(hex.length + 96) {
            append("! 0 ").append(safeDpi).append(' ').append(safeDpi).append(' ')
                .append(mono.height).append(' ').append(copies.coerceIn(1, 99)).append("\r\n")
            append("PW ").append(mono.width).append("\r\n")
            append("EG ")
                .append(mono.rowBytes).append(' ')
                .append(mono.height).append(" 0 0 ")
                .append(hex).append("\r\n")
            append("FORM\r\n")
            append("PRINT\r\n")
        }
        return body.toByteArray(Charsets.US_ASCII)
    }
}
