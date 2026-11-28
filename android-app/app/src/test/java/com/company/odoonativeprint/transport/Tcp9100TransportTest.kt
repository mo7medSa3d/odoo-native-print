package com.company.odoonativeprint.transport

import com.company.odoonativeprint.core.PrinterInfo
import com.company.odoonativeprint.core.PrinterSettings
import com.company.odoonativeprint.core.PrinterTransportType
import org.junit.jupiter.api.Assertions.assertArrayEquals
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import java.net.ServerSocket
import kotlin.concurrent.thread

class Tcp9100TransportTest {

    @Test
    fun `delivers full raw byte stream to local TCP 9100 server`() {
        val payload = ByteArray(40_000) { (it % 251).toByte() }
        ServerSocket(0).use { server ->
            var received = ByteArray(0)
            val reader = thread {
                server.accept().use { client ->
                    received = client.getInputStream().readBytes()
                }
            }
            val printer = PrinterInfo(
                name = "Kitchen",
                transport = PrinterTransportType.TCP_9100,
                address = "127.0.0.1:${server.localPort}",
            )
            val result = Tcp9100Transport().sendRaw(printer, payload, PrinterSettings())
            reader.join(5_000)
            assertTrue(result is TransportResult.Ok)
            assertArrayEquals(payload, received)
        }
    }

    @Test
    fun `returns failure on invalid address`() {
        val printer = PrinterInfo(
            name = "Bad",
            transport = PrinterTransportType.TCP_9100,
            address = "127.0.0.1:99999",
        )
        val result = Tcp9100Transport().sendRaw(printer, byteArrayOf(1), PrinterSettings())
        assertTrue(result is TransportResult.Failure && !result.retryable)
    }

    @Test
    fun `parses scheme case-insensitively and strips trailing path`() {
        val parsed = Tcp9100Transport.parseHostPort("RAW://192.168.1.50:9102/print")
        assertTrue(parsed == ("192.168.1.50" to 9102))
        assertTrue(Tcp9100Transport.parseHostPort("TCP://10.0.0.5") == ("10.0.0.5" to 9100))
    }
}
