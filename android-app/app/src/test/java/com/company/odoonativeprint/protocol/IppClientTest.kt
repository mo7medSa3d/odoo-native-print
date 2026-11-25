package com.company.odoonativeprint.protocol

import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okio.Buffer
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test

class IppClientTest {

    @Test
    fun `sends RFC 8011 Print-Job and parses successful-ok`() {
        MockWebServer().use { server ->
            val okIpp = byteArrayOf(0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x03)
            server.enqueue(MockResponse().setResponseCode(200).setBody(Buffer().write(okIpp)))
            server.start()

            val url = server.url("/ipp/print").toString()
            val req = IppClient.buildPrintJobRequest(url, "%PDF-1.4".toByteArray(), copies = 2, requestId = 7)
            val resp = IppClient.send(url, req)

            assertTrue(resp.isSuccess)
            assertEquals(7, resp.requestId)
            val recorded = server.takeRequest()
            assertEquals("application/ipp", recorded.getHeader("Content-Type"))
        }
    }

    @Test
    fun `flags IPP client-error-not-found as failure`() {
        val errIpp = byteArrayOf(0x02, 0x00, 0x04, 0x06, 0x00, 0x00, 0x00, 0x01, 0x03)
        val parsed = IppClient.parseResponseHeader(errIpp, 200)
        assertFalse(parsed.isSuccess)
        assertEquals(0x0406, parsed.statusCode)
    }
}
