package com.company.odoonativeprint.protocol

import com.company.odoonativeprint.render.MonoBitmap
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test

class RawProtocolEncodersTest {

    private fun sampleMono(): MonoBitmap {
        // 8x2: row0 = black, white, black, white... (0xAA); row1 = all black (0xFF)
        val lum = intArrayOf(
            0, 255, 0, 255, 0, 255, 0, 255,
            0, 0, 0, 0, 0, 0, 0, 0,
        )
        return MonoBitmap.fromLuminance(width = 8, height = 2, luminance = lum, dither = false)
    }

    @Test
    fun `packs 1-bit monochrome rows accurately`() {
        val mono = sampleMono()
        assertEquals(1, mono.rowBytes)
        assertEquals("AAFF", mono.toHexUppercase())
    }

    @Test
    fun `encodes ESC POS raster with init cut and drawer commands`() {
        val bytes = EscPosEncoder.encodeRaster(sampleMono(), cut = true, openDrawer = true)
        assertEquals(0x1B.toByte(), bytes[0])
        assertEquals(0x40.toByte(), bytes[1])
        // GS v 0 0 xL=1 xH=0 yL=2 yH=0
        assertEquals(0x1D.toByte(), bytes[2])
        assertEquals(0x76.toByte(), bytes[3])
        assertEquals(0x30.toByte(), bytes[4])
        assertEquals(0xAA.toByte(), bytes[10])
        assertEquals(0xFF.toByte(), bytes[11])
        // Contains GS V 66 0 (cut) and ESC p 0 25 250 (drawer)
        assertTrue(bytes.size >= 20)
    }

    @Test
    fun `encodes ZPL TSPL EPL and CPCL frames`() {
        val mono = sampleMono()
        val zpl = String(ZplEncoder.encode(mono), Charsets.US_ASCII)
        assertTrue(zpl.startsWith("^XA"))
        assertTrue(zpl.contains("^GFA,2,2,1,AAFF^FS"))
        assertTrue(zpl.trimEnd().endsWith("^XZ"))

        val tspl = String(TsplEncoder.encode(mono, 100, 150), Charsets.ISO_8859_1)
        assertTrue(tspl.contains("SIZE 100 mm, 150 mm"))
        assertTrue(tspl.contains("BITMAP 0,0,1,2,0,"))
        assertTrue(tspl.contains("PRINT 1,1"))

        val eplBytes = EplEncoder.encode(mono)
        val epl = String(eplBytes, Charsets.ISO_8859_1)
        assertTrue(epl.startsWith("N\r\nq8\r\nQ2,24\r\nGW0,0,1,2,"))
        assertTrue(epl.endsWith("\r\nP1\r\n"))
        // Inverted raster bytes: 0xAA -> 0x55, 0xFF -> 0x00
        val headerLen = "N\r\nq8\r\nQ2,24\r\nGW0,0,1,2,".length
        assertEquals(0x55.toByte(), eplBytes[headerLen])
        assertEquals(0x00.toByte(), eplBytes[headerLen + 1])

        val eplCustomGap = String(EplEncoder.encode(mono, copies = 3, gapDots = 36), Charsets.ISO_8859_1)
        assertTrue(eplCustomGap.startsWith("N\r\nq8\r\nQ2,36\r\nGW0,0,1,2,"))
        assertTrue(eplCustomGap.endsWith("\r\nP3\r\n"))

        val cpcl = String(CpclEncoder.encode(mono), Charsets.US_ASCII)
        assertTrue(cpcl.startsWith("! 0 200 200 2 1"))
        assertTrue(cpcl.contains("EG 1 2 0 0 AAFF"))
        assertTrue(cpcl.trimEnd().endsWith("PRINT"))
    }

    @Test
    fun `printer settings dithering defaults to false for crisp receipt text`() {
        val defaults = com.company.odoonativeprint.core.PrinterSettings()
        assertFalse(defaults.dithering)
        val parsed = com.company.odoonativeprint.core.PrinterSettings.fromJson(org.json.JSONObject())
        assertFalse(parsed.dithering)
    }
}
