package com.company.odoonativeprint.transport

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothSocket
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.content.ContextCompat
import com.company.odoonativeprint.core.PrinterInfo
import com.company.odoonativeprint.core.PrinterSettings
import java.io.IOException
import java.util.UUID
import java.util.concurrent.Executors
import java.util.concurrent.ScheduledExecutorService
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Classic Bluetooth SPP (RFCOMM) transport for portable & POS thermal/label printers.
 */
class BluetoothSppTransport(private val context: Context) : RawByteTransport {

    @SuppressLint("MissingPermission")
    override fun sendRaw(
        printer: PrinterInfo,
        bytes: ByteArray,
        settings: PrinterSettings,
    ): TransportResult {
        if (!hasConnectPermission()) {
            return TransportResult.Failure(
                "BT_PERMISSION_DENIED",
                "Bluetooth Connect permission is required",
                retryable = false,
            )
        }
        val adapter = context.getSystemService(BluetoothManager::class.java)?.adapter
            ?: return TransportResult.Failure("BT_UNAVAILABLE", "Bluetooth adapter unavailable", retryable = false)
        if (!adapter.isEnabled) {
            return TransportResult.Failure("BT_DISABLED", "Bluetooth is turned off", retryable = true)
        }

        val mac = printer.address.removePrefix("bt://").trim()
        val device = try {
            adapter.getRemoteDevice(mac)
        } catch (e: IllegalArgumentException) {
            return TransportResult.Failure("BT_INVALID_MAC", "Invalid Bluetooth MAC: $mac", retryable = false)
        }

        var socket: BluetoothSocket? = null
        val timedOut = AtomicBoolean(false)
        return try {
            // Cancel active BT discovery before RFCOMM connect to avoid severe slowdown/failure.
            runCatching { adapter.cancelDiscovery() }
            val primary = device.createRfcommSocketToServiceRecord(SPP_UUID)
            socket = primary
            val activeSocket = try {
                withSocketTimeout(primary, CONNECT_TIMEOUT_MS, timedOut) {
                    primary.connect()
                }
                primary
            } catch (e: IOException) {
                runCatching { primary.close() }
                if (timedOut.get()) throw e
                val fallback = device.createInsecureRfcommSocketToServiceRecord(SPP_UUID)
                socket = fallback
                withSocketTimeout(fallback, CONNECT_TIMEOUT_MS, timedOut) {
                    fallback.connect()
                }
                fallback
            }
            withSocketTimeout(activeSocket, WRITE_TIMEOUT_MS, timedOut) {
                val out = activeSocket.outputStream
                val pacingMs = settings.bluetoothPacingMs.toLong()
                var offset = 0
                while (offset < bytes.size) {
                    val len = minOf(CHUNK_SIZE, bytes.size - offset)
                    out.write(bytes, offset, len)
                    offset += len
                    if (pacingMs > 0L) Thread.sleep(pacingMs)
                }
                out.flush()
            }
            // Allow cheap thermal printers to drain the final cut command before RFCOMM disconnect
            Thread.sleep(250L)
            TransportResult.Ok
        } catch (e: IOException) {
            if (timedOut.get()) {
                TransportResult.Failure("BT_TIMEOUT", "Bluetooth printer timed out", retryable = true)
            } else {
                TransportResult.Failure("BT_IO_ERROR", e.message ?: "Bluetooth write failed", retryable = true)
            }
        } finally {
            runCatching { socket?.close() }
        }
    }

    private inline fun withSocketTimeout(
        socket: BluetoothSocket,
        timeoutMs: Long,
        timedOut: AtomicBoolean,
        block: () -> Unit,
    ) {
        val future = watchdog.schedule(
            {
                timedOut.set(true)
                runCatching { socket.close() }
            },
            timeoutMs,
            TimeUnit.MILLISECONDS,
        )
        try {
            block()
        } finally {
            future.cancel(false)
        }
    }

    fun hasConnectPermission(): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return true
        return ContextCompat.checkSelfPermission(context, Manifest.permission.BLUETOOTH_CONNECT) ==
            PackageManager.PERMISSION_GRANTED
    }

    companion object {
        val SPP_UUID: UUID = UUID.fromString("00001101-0000-1000-8000-00805F9B34FB")
        private const val CHUNK_SIZE = 512
        private const val CONNECT_TIMEOUT_MS = 10_000L
        private const val WRITE_TIMEOUT_MS = 30_000L
        private val watchdog: ScheduledExecutorService = Executors.newSingleThreadScheduledExecutor { r ->
            Thread(r, "bt-spp-watchdog").apply { isDaemon = true }
        }
    }
}
