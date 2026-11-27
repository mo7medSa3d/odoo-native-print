package com.company.odoonativeprint.protocol

import com.company.odoonativeprint.render.MonoBitmap
import java.io.ByteArrayOutputStream

/**
 * Pure-Kotlin EPL2 encoder matching crates/print-raw/src/epl.rs.
 * EPL GW uses 0 = black dot, 1 = white dot.
 */
object EplEncoder {
    fun encode(mono: MonoBitmap, copies: Int = 1, gapDots: Int = 24): ByteArray {
        val out = ByteArrayOutputStream(mono.data.size + 96)
        out.write("N\r\n".toByteArray(Charsets.US_ASCII))
        out.write("q${mono.width.coerceAtLeast(8)}\r\n".toByteArray(Charsets.US_ASCII))
        out.write("Q${mono.height.coerceAtLeast(1)},${gapDots.coerceAtLeast(0)}\r\n".toByteArray(Charsets.US_ASCII))
        out.write("GW0,0,${mono.rowBytes},${mono.height},".toByteArray(Charsets.US_ASCII))
        out.write(mono.invertedCopy())
        out.write("\r\nP${copies.coerceIn(1, 99)}\r\n".toByteArray(Charsets.US_ASCII))
        return out.toByteArray()
    }
}
