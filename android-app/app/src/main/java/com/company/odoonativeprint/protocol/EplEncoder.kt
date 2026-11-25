package com.company.odoonativeprint.protocol

import com.company.odoonativeprint.render.MonoBitmap
import java.io.ByteArrayOutputStream

/**
 * Pure-Kotlin EPL2 encoder matching crates/print-raw/src/epl.rs.
 * EPL GW uses 0 = black dot, 1 = white dot.
 */
object EplEncoder {
    fun encode(mono: MonoBitmap, copies: Int = 1): ByteArray {
        val out = ByteArrayOutputStream(mono.data.size + 64)
        out.write("\nN\n".toByteArray(Charsets.US_ASCII))
        out.write("q${mono.width}\n".toByteArray(Charsets.US_ASCII))
        out.write("GW0,0,${mono.rowBytes},${mono.height},".toByteArray(Charsets.US_ASCII))
        out.write(mono.invertedCopy())
        out.write("\nP${copies.coerceIn(1, 99)}\n".toByteArray(Charsets.US_ASCII))
        return out.toByteArray()
    }
}
