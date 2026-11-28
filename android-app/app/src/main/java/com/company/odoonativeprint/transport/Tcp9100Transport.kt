package com.company.odoonativeprint.transport

import com.company.odoonativeprint.core.PrinterInfo
import com.company.odoonativeprint.core.PrinterSettings
import java.io.IOException
import java.net.InetSocketAddress
import java.net.Socket
import java.net.SocketTimeoutException

/**
 * Raw TCP 9100 transport with strict connect, write, and drain deadlines.
 */
class Tcp9100Transport(
    private val connectTimeoutMs: Int = 5_000,
    private val ioTimeoutMs: Int = 15_000,
    private val chunkSize: Int = 16_384,
) : RawByteTransport {

    override fun sendRaw(
        printer: PrinterInfo,
        bytes: ByteArray,
        settings: PrinterSettings,
    ): TransportResult {
        val (host, port) = parseHostPort(printer.address)
            ?: return TransportResult.Failure("INVALID_ADDRESS", "Invalid TCP address: ${printer.address}", retryable = false)

        return try {
            Socket().use { socket ->
                socket.tcpNoDelay = true
                socket.soTimeout = ioTimeoutMs
                socket.connect(InetSocketAddress(host, port), connectTimeoutMs)
                val out = socket.getOutputStream()
                var offset = 0
                while (offset < bytes.size) {
                    val len = minOf(chunkSize, bytes.size - offset)
                    out.write(bytes, offset, len)
                    offset += len
                }
                out.flush()
                runCatching { socket.shutdownOutput() }
            }
            TransportResult.Ok
        } catch (e: SocketTimeoutException) {
            TransportResult.Failure("PRINTER_TIMEOUT", "TCP timeout talking to $host:$port", retryable = true)
        } catch (e: IOException) {
            TransportResult.Failure("TCP_IO_ERROR", e.message ?: "TCP error talking to $host:$port", retryable = true)
        }
    }

    fun isReachable(address: String, timeoutMs: Int = 1_500): Boolean {
        val (host, port) = parseHostPort(address) ?: return false
        return try {
            Socket().use { it.connect(InetSocketAddress(host, port), timeoutMs) }
            true
        } catch (_: Exception) {
            false
        }
    }

    companion object {
        fun parseHostPort(address: String): Pair<String, Int>? {
            val trimmed = address.trim()
            val schemeEnd = trimmed.indexOf("://")
            val withoutScheme = if (schemeEnd > 0) {
                val scheme = trimmed.substring(0, schemeEnd).lowercase()
                if (scheme == "tcp" || scheme == "raw" || scheme == "socket") trimmed.substring(schemeEnd + 3) else trimmed
            } else {
                trimmed
            }
            val clean = withoutScheme.substringBefore('/').trim()
            if (clean.isEmpty()) return null
            val idx = clean.lastIndexOf(':')
            return if (idx > 0 && clean.indexOf(']') < idx) {
                val host = clean.substring(0, idx).removePrefix("[").removeSuffix("]")
                val port = clean.substring(idx + 1).toIntOrNull() ?: return null
                if (host.isEmpty() || port !in 1..65535) return null
                host to port
            } else {
                val host = clean.removePrefix("[").removeSuffix("]")
                if (host.isEmpty()) return null
                host to 9100
            }
        }
    }
}
