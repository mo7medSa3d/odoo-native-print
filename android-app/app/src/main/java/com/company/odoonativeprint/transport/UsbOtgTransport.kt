package com.company.odoonativeprint.transport

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.hardware.usb.UsbConstants
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbEndpoint
import android.hardware.usb.UsbInterface
import android.hardware.usb.UsbManager
import android.os.Build
import com.company.odoonativeprint.core.PrinterInfo
import com.company.odoonativeprint.core.PrinterSettings

/**
 * USB Host (OTG) bulk OUT transport for USB Printer Class (0x07) thermal & label printers.
 */
class UsbOtgTransport(private val context: Context) : RawByteTransport {

    override fun sendRaw(
        printer: PrinterInfo,
        bytes: ByteArray,
        settings: PrinterSettings,
    ): TransportResult {
        val manager = context.getSystemService(Context.USB_SERVICE) as? UsbManager
            ?: return TransportResult.Failure("USB_UNAVAILABLE", "USB Host service unavailable", retryable = false)

        val deviceName = printer.address.removePrefix("usb://")
        val device = manager.deviceList[deviceName]
            ?: manager.deviceList.values.firstOrNull { isPrinterDevice(it) && formatKey(it) == deviceName }
            ?: return TransportResult.Failure("USB_DISCONNECTED", "USB printer not connected", retryable = true)

        if (!manager.hasPermission(device)) {
            return TransportResult.Failure(
                "USB_PERMISSION_DENIED",
                "USB permission required for ${printer.displayName}",
                retryable = false,
            )
        }

        val (usbInterface, outEndpoint) = findPrinterBulkOut(device)
            ?: return TransportResult.Failure("USB_NO_BULK_OUT", "No USB bulk OUT endpoint found", retryable = false)

        val conn = manager.openDevice(device)
            ?: return TransportResult.Failure("USB_OPEN_FAILED", "Could not open USB device", retryable = true)

        return try {
            if (!conn.claimInterface(usbInterface, true)) {
                return TransportResult.Failure("USB_CLAIM_FAILED", "Could not claim USB printer interface", retryable = true)
            }
            var offset = 0
            while (offset < bytes.size) {
                val len = minOf(CHUNK_SIZE, bytes.size - offset)
                val chunk = bytes.copyOfRange(offset, offset + len)
                val written = conn.bulkTransfer(outEndpoint, chunk, len, TRANSFER_TIMEOUT_MS)
                if (written <= 0) {
                    return TransportResult.Failure("USB_TRANSFER_FAILED", "USB bulk transfer failed at byte $offset", retryable = true)
                }
                offset += written
            }
            TransportResult.Ok
        } finally {
            runCatching { conn.releaseInterface(usbInterface) }
            runCatching { conn.close() }
        }
    }

    companion object {
        const val ACTION_USB_PERMISSION = "com.company.odoonativeprint.USB_PERMISSION"
        private const val CHUNK_SIZE = 4_096
        private const val TRANSFER_TIMEOUT_MS = 10_000

        /**
         * Requests USB permission using an explicit Intent + FLAG_MUTABLE (required on Android 14+).
         */
        fun requestPermission(context: Context, usbAddress: String): Boolean {
            val manager = context.getSystemService(Context.USB_SERVICE) as? UsbManager ?: return false
            val deviceName = usbAddress.removePrefix("usb://")
            val device = manager.deviceList[deviceName]
                ?: manager.deviceList.values.firstOrNull { isPrinterDevice(it) && formatKey(it) == deviceName }
                ?: return false
            if (manager.hasPermission(device)) return true

            val intent = Intent(ACTION_USB_PERMISSION).setPackage(context.packageName)
            val flags = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_MUTABLE
            } else {
                PendingIntent.FLAG_UPDATE_CURRENT
            }
            val pi = PendingIntent.getBroadcast(context, device.deviceId, intent, flags)
            manager.requestPermission(device, pi)
            return true
        }

        fun isPrinterDevice(device: UsbDevice): Boolean = findPrinterBulkOut(device) != null

        fun formatKey(device: UsbDevice): String = "${device.vendorId}:${device.productId}:${device.deviceId}"

        fun findPrinterBulkOut(device: UsbDevice): Pair<UsbInterface, UsbEndpoint>? {
            for (i in 0 until device.interfaceCount) {
                val iface = device.getInterface(i)
                val isPrinterIface = iface.interfaceClass == UsbConstants.USB_CLASS_PRINTER ||
                    device.deviceClass == UsbConstants.USB_CLASS_PRINTER
                if (!isPrinterIface) continue
                for (e in 0 until iface.endpointCount) {
                    val ep = iface.getEndpoint(e)
                    if (ep.type == UsbConstants.USB_ENDPOINT_XFER_BULK && ep.direction == UsbConstants.USB_DIR_OUT) {
                        return iface to ep
                    }
                }
            }
            return null
        }
    }
}
