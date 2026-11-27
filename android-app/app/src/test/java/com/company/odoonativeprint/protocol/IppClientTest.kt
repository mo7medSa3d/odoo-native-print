package com.company.odoonativeprint.protocol

import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okio.Buffer
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.DataOutputStream
import java.io.File

class IppClientTest {

    @Test
    fun `sends RFC 8011 IPP 1_1 Print-Job and parses successful-ok`() {
        MockWebServer().use { server ->
            val okIpp = byteArrayOf(0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x03)
            server.enqueue(MockResponse().setResponseCode(200).setBody(Buffer().write(okIpp)))
            server.start()

            val url = server.url("/ipp/print").toString()
            val req = IppClient.buildPrintJobRequest(url, "%PDF-1.4".toByteArray(), copies = 2, requestId = 7)
            assertEquals(0x01.toByte(), req[0])
            assertEquals(0x01.toByte(), req[1])
            val resp = IppClient.send(url, req)

            assertTrue(resp.isSuccess)
            assertEquals(7, resp.requestId)
            val recorded = server.takeRequest()
            assertEquals("application/ipp", recorded.getHeader("Content-Type"))
        }
    }

    @Test
    fun `omits empty job-attributes group when copies is 1 and media and sides are blank`() {
        val getAttrs = IppClient.buildGetPrinterAttributesRequest("ipp://127.0.0.1:631/ipp/print")
        assertEquals(0x01.toByte(), getAttrs[0])
        assertEquals(0x01.toByte(), getAttrs[1])

        val doc = "%PDF-1.4".toByteArray()
        val req = IppClient.buildPrintJobRequest(
            printerUri = "ipp://127.0.0.1:631/ipp/print",
            documentBytes = doc,
            copies = 1,
            media = "   ",
            sides = "",
        )
        // Before the document payload, the last byte of the IPP header is TAG_END_OF_ATTRIBUTES (0x03)
        // and no TAG_JOB_ATTRIBUTES (0x02) delimiter precedes it.
        val header = req.copyOfRange(0, req.size - doc.size)
        assertEquals(0x03.toByte(), header.last())
        assertTrue(String(header, Charsets.US_ASCII).endsWith("application/pdf\u0003"))
    }

    @Test
    fun `decodes withLanguage compound values and skips out-of-band tags`() {
        val baos = ByteArrayOutputStream()
        val out = DataOutputStream(baos)
        // IPP 1.1 successful-ok, requestId=1, printer-attributes-tag (0x04)
        out.writeByte(1); out.writeByte(1)
        out.writeShort(0x0000)
        out.writeInt(1)
        out.writeByte(0x04)

        // nameWithLanguage (0x36) for printer-name: lang="en", name="Office Laser"
        val nameKey = "printer-name".toByteArray(Charsets.UTF_8)
        val lang = "en".toByteArray(Charsets.UTF_8)
        val nameVal = "Office Laser".toByteArray(Charsets.UTF_8)
        out.writeByte(0x36)
        out.writeShort(nameKey.size); out.write(nameKey)
        out.writeShort(2 + lang.size + 2 + nameVal.size)
        out.writeShort(lang.size); out.write(lang)
        out.writeShort(nameVal.size); out.write(nameVal)

        // textWithLanguage (0x35) for media-supported: lang="en-us", text="iso_a4_210x297mm"
        val mediaKey = "media-supported".toByteArray(Charsets.UTF_8)
        val lang2 = "en-us".toByteArray(Charsets.UTF_8)
        val mediaVal = "iso_a4_210x297mm".toByteArray(Charsets.UTF_8)
        out.writeByte(0x35)
        out.writeShort(mediaKey.size); out.write(mediaKey)
        out.writeShort(2 + lang2.size + 2 + mediaVal.size)
        out.writeShort(lang2.size); out.write(lang2)
        out.writeShort(mediaVal.size); out.write(mediaVal)

        // Out-of-band no-value (0x13) for color-supported and sides-supported must be skipped
        val colorKey = "color-supported".toByteArray(Charsets.UTF_8)
        out.writeByte(0x13)
        out.writeShort(colorKey.size); out.write(colorKey)
        out.writeShort(0)

        val sidesKey = "sides-supported".toByteArray(Charsets.UTF_8)
        out.writeByte(0x13)
        out.writeShort(sidesKey.size); out.write(sidesKey)
        out.writeShort(0)

        out.writeByte(0x03) // end-of-attributes
        out.flush()

        val attrs = IppClient.parsePrinterAttributesResponse(baos.toByteArray())
        assertEquals("Office Laser", attrs.printerName)
        assertEquals(listOf("iso_a4_210x297mm"), attrs.mediaSupported)
        assertNull(attrs.colorSupported)
        assertTrue(attrs.sidesSupported.isEmpty())
    }

    @Test
    fun `caps HTTP response reads to MAX_IPP_RESPONSE_BYTES`() {
        assertEquals(1024 * 1024, IppClient.MAX_IPP_RESPONSE_BYTES)
        val oversized = ByteArray(4096) { 0x41 }
        val capped = IppClient.readCappedBytes(ByteArrayInputStream(oversized), maxBytes = 1024)
        assertEquals(1024, capped.size)
    }

    @Test
    fun `flags IPP client-error-not-found as failure`() {
        val errIpp = byteArrayOf(0x01, 0x01, 0x04, 0x06, 0x00, 0x00, 0x00, 0x01, 0x03)
        val parsed = IppClient.parseResponseHeader(errIpp, 200)
        assertFalse(parsed.isSuccess)
        assertEquals(0x0406, parsed.statusCode)
    }

    @Test
    fun `streams IPP header and PDF file with exact fixed content length`() {
        MockWebServer().use { server ->
            val okIpp = byteArrayOf(0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x09, 0x03)
            server.enqueue(MockResponse().setResponseCode(200).setBody(Buffer().write(okIpp)))
            server.start()

            val pdfBytes = "%PDF-1.4\n%%EOF\n".toByteArray()
            val tmp = File.createTempFile("ipp-stream-test", ".pdf").apply {
                writeBytes(pdfBytes)
                deleteOnExit()
            }
            val url = server.url("/ipp/print").toString()
            val header = IppClient.buildPrintJobHeader(url, copies = 1, requestId = 9)
            val resp = IppClient.sendFile(url, header, tmp)

            assertTrue(resp.isSuccess)
            assertEquals(9, resp.requestId)
            val recorded = server.takeRequest()
            assertEquals("application/ipp", recorded.getHeader("Content-Type"))
            assertEquals((header.size + pdfBytes.size).toString(), recorded.getHeader("Content-Length"))
            assertEquals(header.size + pdfBytes.size, recorded.body.size.toInt())
        }
    }

    @Test
    fun `normalizes ipp ipps and schemeless urls`() {
        assertEquals("http://192.168.1.50:631/ipp/print", IppClient.normalizeHttpUrl("ipp://192.168.1.50:631/ipp/print"))
        assertEquals("https://printer.local:443/ipp/print", IppClient.normalizeHttpUrl("ipps://printer.local:443/ipp/print"))
        assertEquals("http://[::1]:631/ipp/print", IppClient.normalizeHttpUrl("[::1]:631/ipp/print"))
    }
}
