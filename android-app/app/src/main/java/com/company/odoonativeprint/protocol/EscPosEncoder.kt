package com.company.odoonativeprint.protocol

import com.company.odoonativeprint.render.MonoBitmap
import java.io.ByteArrayOutputStream

/**
 * Pure-Kotlin ESC/POS encoder matching crates/print-raw/src/escpos.rs.
 * Chunks tall receipts into <= 256-row bands so low-RAM Bluetooth/USB thermal printers never overflow.
 */
object EscPosEncoder {

    private const val MAX_BAND_ROWS = 256

    fun encodeRaster(
        mono: MonoBitmap,
        cut: Boolean = true,
        openDrawer: Boolean = false,
        feedLines: Int = 4,
    ): ByteArray {
        val out = ByteArrayOutputStream(mono.data.size + 128)
        // ESC @ (Initialize printer)
        out.write(byteArrayOf(0x1B, 0x40))

        var y = 0
        while (y < mono.height) {
            val bandHeight = minOf(MAX_BAND_ROWS, mono.height - y)
            val xL = mono.rowBytes and 0xFF
            val xH = (mono.rowBytes ushr 8) and 0xFF
            val yL = bandHeight and 0xFF
            val yH = (bandHeight ushr 8) and 0xFF

            // GS v 0 m xL xH yL yH (raster bit image, normal mode m=0)
            out.write(byteArrayOf(0x1D, 0x76, 0x30, 0x00, xL.toByte(), xH.toByte(), yL.toByte(), yH.toByte()))
            val offset = y * mono.rowBytes
            out.write(mono.data, offset, bandHeight * mono.rowBytes)
            y += bandHeight
        }

        if (feedLines > 0) {
            // ESC d n (Print and feed n lines)
            out.write(byteArrayOf(0x1B, 0x64, feedLines.coerceIn(1, 20).toByte()))
        }
        if (cut) {
            // GS V 66 0 (Feed and partial cut)
            out.write(byteArrayOf(0x1D, 0x56, 0x42, 0x00))
        }
        if (openDrawer) {
            // ESC p 0 25 250 (Cash drawer kick pin 2)
            out.write(byteArrayOf(0x1B, 0x70, 0x00, 0x19, 0xFA.toByte()))
        }
        return out.toByteArray()
    }
}
