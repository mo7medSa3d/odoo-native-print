package com.company.odoonativeprint.transport

import android.annotation.SuppressLint
import android.bluetooth.BluetoothClass
import android.bluetooth.BluetoothManager
import android.content.Context
import android.hardware.usb.UsbManager
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import com.company.odoonativeprint.core.AppConfigStore
import com.company.odoonativeprint.core.PrinterCapabilities
import com.company.odoonativeprint.core.PrinterInfo
import com.company.odoonativeprint.core.PrinterProtocol
import com.company.odoonativeprint.core.PrinterTransportType
import com.company.odoonativeprint.protocol.IppClient
import com.company.odoonativeprint.protocol.IppPrinterAttributes
import java.net.Inet4Address
import java.util.concurrent.CompletableFuture
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicLong

/**
 * Pure-JVM 12-second TTL single-source discovery cache with force invalidation
 * and monotonic capsEpoch (mirrors the desktop stale-printer-cache fix).
 */
class PrinterDiscoveryCache(
    private val ttlMs: Long = 12_000L,
    private val clock: () -> Long = { System.currentTimeMillis() },
    private val probe: () -> List<PrinterInfo>,
) {
    private val lock = Any()
    private var cached: List<PrinterInfo>? = null
    private var cachedAtMs: Long = 0L
    private val epoch = AtomicLong(1L)

    fun capsEpoch(): Long = epoch.get()

    fun get(force: Boolean = false): List<PrinterInfo> = synchronized(lock) {
        val now = clock()
        val snap = cached
        if (!force && snap != null && (now - cachedAtMs) < ttlMs) {
            return snap
        }
        val fresh = probe()
        cached = fresh
        cachedAtMs = now
        if (force || snap == null) {
            epoch.incrementAndGet()
        }
        return fresh
    }

    fun invalidate() = synchronized(lock) {
        cached = null
        cachedAtMs = 0L
        epoch.incrementAndGet()
    }
}

/**
 * Unified Android printer discovery aggregating:
 * 1. User-configured manual TCP 9100 / IPP / BT printers
 * 2. Live mDNS/NSD network printers (_ipp._tcp., _pdl-datastream._tcp.) — API 34+ ServiceInfoCallback + legacy fallback
 * 3. Paired Classic Bluetooth printers
 * 4. Connected USB OTG Printer Class (0x07) devices
 * 5. Optional Android System Print fallback
 */
class PrinterDiscoveryService(
    private val context: Context,
    private val configStore: AppConfigStore,
    private val tcpTransport: Tcp9100Transport,
    private val btTransport: BluetoothSppTransport,
) {
    private val nsdDiscovered = ConcurrentHashMap<String, PrinterInfo>()
    private val nsdListeners = mutableListOf<NsdManager.DiscoveryListener>()
    private val nsdInfoCallbacks = ConcurrentHashMap<String, Any>()
    private val nsdExecutor = Executors.newSingleThreadExecutor()
    private val probeExecutor = Executors.newCachedThreadPool()
    private val nsdManager: NsdManager? = context.getSystemService(Context.NSD_SERVICE) as? NsdManager
    private val multicastLock: WifiManager.MulticastLock? = runCatching {
        (context.applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager)
            ?.createMulticastLock("itbaa-mdns")
            ?.apply { setReferenceCounted(false) }
    }.getOrNull()
    private val ippCapsCache = ConcurrentHashMap<String, Pair<Long, IppPrinterAttributes>>()

    private val cache = PrinterDiscoveryCache(probe = ::probeAll)

    fun capsEpoch(): Long = cache.capsEpoch()

    fun listPrinters(force: Boolean = false): List<PrinterInfo> = cache.get(force)

    fun findPrinter(name: String): PrinterInfo? {
        val needle = name.trim()
        val bare = needle.removePrefix("tcp://")
        fun List<PrinterInfo>.match(): PrinterInfo? =
            firstOrNull { it.name.equals(needle, ignoreCase = true) }
                ?: firstOrNull { it.address.removePrefix("tcp://").equals(bare, ignoreCase = true) }
        return listPrinters(force = false).match() ?: listPrinters(force = true).match()
    }

    fun invalidate() {
        cache.invalidate()
    }

    fun capabilitiesFor(printerName: String): PrinterCapabilities? {
        val printer = findPrinter(printerName) ?: return null
        val settings = configStore.getPrinterSettings(printer.name)
        val effectiveProtocol = if (settings.protocol == PrinterProtocol.AUTO) printer.protocol else settings.protocol
        val ippAttrs = if (printer.transport == PrinterTransportType.IPP) {
            cachedIppAttributes(printer.address)
        } else {
            null
        }
        val forms = ippAttrs?.mediaSupported
            ?.mapNotNull(::formatIppMediaLabel)
            ?.distinct()
            ?.takeIf { it.isNotEmpty() }
            ?: when (effectiveProtocol) {
                PrinterProtocol.ESCPOS -> listOf("80mm Roll", "58mm Roll")
                PrinterProtocol.ZPL, PrinterProtocol.TSPL, PrinterProtocol.EPL, PrinterProtocol.CPCL ->
                    listOf("100x150mm", "100x100mm", "50x30mm", "40x25mm")
                else -> listOf("A4", "Letter", "80mm Roll", "100x150mm")
            }
        return PrinterCapabilities(
            printer = printer.name,
            transport = printer.transport,
            protocol = effectiveProtocol,
            dpi = settings.dpi,
            paperWidthMm = settings.paperWidthMm,
            forms = forms,
            supportsColor = ippAttrs?.colorSupported
                ?: (printer.transport == PrinterTransportType.IPP || printer.transport == PrinterTransportType.ANDROID_SYSTEM),
            supportsDuplex = ippAttrs?.sidesSupported?.any { it.startsWith("two-sided") }
                ?: (printer.transport == PrinterTransportType.IPP),
            supportsRaw = printer.transport != PrinterTransportType.ANDROID_SYSTEM,
            supportsPdf = true,
            supportsCut = effectiveProtocol == PrinterProtocol.ESCPOS,
            supportsDrawer = effectiveProtocol == PrinterProtocol.ESCPOS,
            capsEpoch = capsEpoch(),
        )
    }

    private fun cachedIppAttributes(address: String): IppPrinterAttributes? {
        val now = System.currentTimeMillis()
        val hit = ippCapsCache[address]
        if (hit != null && (now - hit.first) < 60_000L) {
            return hit.second
        }
        val fresh = IppClient.getPrinterAttributes(address) ?: return hit?.second
        ippCapsCache[address] = now to fresh
        return fresh
    }

    private fun formatIppMediaLabel(media: String): String? {
        val lower = media.trim().lowercase()
        return when {
            lower.isEmpty() -> null
            lower.contains("a4") -> "A4"
            lower.contains("a5") -> "A5"
            lower.contains("letter") -> "Letter"
            lower.contains("legal") -> "Legal"
            lower.contains("roll") && lower.contains("80") -> "80mm Roll"
            lower.contains("roll") && lower.contains("58") -> "58mm Roll"
            else -> media.trim()
        }
    }

    @Synchronized
    fun startMdnsDiscovery() {
        val mgr = nsdManager ?: return
        stopMdnsDiscovery()
        listOf("_ipp._tcp.", "_pdl-datastream._tcp.").forEach { serviceType ->
            val listener = object : NsdManager.DiscoveryListener {
                override fun onDiscoveryStarted(regType: String) = Unit
                override fun onServiceFound(service: NsdServiceInfo) {
                    resolveNsdService(mgr, service, serviceType)
                }

                override fun onServiceLost(service: NsdServiceInfo) {
                    nsdDiscovered.remove(service.serviceName)
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                        (nsdInfoCallbacks.remove(service.serviceName) as? NsdManager.ServiceInfoCallback)?.let { cb ->
                            runCatching { mgr.unregisterServiceInfoCallback(cb) }
                        }
                    }
                    cache.invalidate()
                }

                override fun onDiscoveryStopped(serviceType: String) = Unit
                override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) = Unit
                override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) = Unit
            }
            runCatching {
                mgr.discoverServices(serviceType, NsdManager.PROTOCOL_DNS_SD, listener)
                nsdListeners += listener
            }
        }
        if (nsdListeners.isNotEmpty()) {
            runCatching { multicastLock?.acquire() }
        }
    }

    private fun resolveNsdService(mgr: NsdManager, service: NsdServiceInfo, serviceType: String) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            val cb = object : NsdManager.ServiceInfoCallback {
                override fun onServiceInfoCallbackRegistrationFailed(errorCode: Int) = Unit
                override fun onServiceUpdated(info: NsdServiceInfo) {
                    val host = (info.hostAddresses.firstOrNull { it is Inet4Address } ?: info.hostAddresses.firstOrNull())?.hostAddress ?: return
                    val pdl = info.attributes["pdl"]?.decodeToString().orEmpty()
                    val ty = info.attributes["ty"]?.decodeToString().orEmpty()
                    recordResolvedService(info.serviceName, host, info.port, serviceType, "$pdl $ty")
                }

                override fun onServiceLost() {
                    nsdDiscovered.remove(service.serviceName)
                    cache.invalidate()
                }

                override fun onServiceInfoCallbackUnregistered() = Unit
            }
            (nsdInfoCallbacks.put(service.serviceName, cb) as? NsdManager.ServiceInfoCallback)?.let { prev ->
                runCatching { mgr.unregisterServiceInfoCallback(prev) }
            }
            runCatching { mgr.registerServiceInfoCallback(service, nsdExecutor, cb) }
                .onFailure { nsdInfoCallbacks.remove(service.serviceName, cb) }
        } else {
            @Suppress("DEPRECATION")
            mgr.resolveService(service, object : NsdManager.ResolveListener {
                override fun onResolveFailed(info: NsdServiceInfo, errorCode: Int) = Unit
                override fun onServiceResolved(info: NsdServiceInfo) {
                    @Suppress("DEPRECATION")
                    val host = info.host?.hostAddress ?: return
                    recordResolvedService(info.serviceName, host, info.port, serviceType)
                }
            })
        }
    }

    private fun recordResolvedService(
        serviceName: String,
        host: String,
        port: Int,
        serviceType: String,
        txtHints: String = "",
    ) {
        val isIpp = serviceType.startsWith("_ipp")
        val transport = if (isIpp) PrinterTransportType.IPP else PrinterTransportType.TCP_9100
        val uriHost = if (host.contains(':')) "[$host]" else host
        val address = if (isIpp) "ipp://$uriHost:$port/ipp/print" else "$uriHost:$port"
        val hint = "${serviceName.lowercase()} ${txtHints.lowercase()}"
        val protocol = when {
            isIpp -> PrinterProtocol.PDF
            hint.contains("zpl") || hint.contains("zebra") -> PrinterProtocol.ZPL
            hint.contains("tspl") || hint.contains("tsc") -> PrinterProtocol.TSPL
            hint.contains("epl") -> PrinterProtocol.EPL
            hint.contains("cpcl") -> PrinterProtocol.CPCL
            else -> PrinterProtocol.ESCPOS
        }
        val name = serviceName.trim().ifEmpty { "Network Printer ($host)" }
        nsdDiscovered[name] = PrinterInfo(
            name = name,
            displayName = name,
            transport = transport,
            address = address,
            protocol = protocol,
            online = true,
            status = "ready",
        )
        cache.invalidate()
    }

    @Synchronized
    fun stopMdnsDiscovery() {
        val mgr = nsdManager ?: return
        runCatching { if (multicastLock?.isHeld == true) multicastLock.release() }
        nsdListeners.forEach { runCatching { mgr.stopServiceDiscovery(it) } }
        nsdListeners.clear()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            nsdInfoCallbacks.values.forEach { cb ->
                (cb as? NsdManager.ServiceInfoCallback)?.let { runCatching { mgr.unregisterServiceInfoCallback(it) } }
            }
            nsdInfoCallbacks.clear()
        }
    }

    @SuppressLint("MissingPermission")
    private fun probeAll(): List<PrinterInfo> {
        val snap = configStore.current()
        val byName = LinkedHashMap<String, PrinterInfo>()

        // 1. Manual configured TCP/IPP/BT printers (probe TCP endpoints in parallel so offline printers don't stall UI)
        val tcpReachability = snap.manualPrinters
            .filter { it.transport == PrinterTransportType.TCP_9100 }
            .associate { m ->
                m.name to CompletableFuture.supplyAsync(
                    { tcpTransport.isReachable(m.address, timeoutMs = 500) },
                    probeExecutor,
                )
            }
        for (m in snap.manualPrinters) {
            val online = tcpReachability[m.name]?.get() ?: true
            byName[m.name] = PrinterInfo(
                name = m.name,
                displayName = m.name,
                transport = m.transport,
                address = m.address,
                protocol = m.protocol,
                online = online,
                status = if (online) "ready" else "offline",
                isDefault = snap.defaultPrinter == m.name,
                dpi = m.dpi,
                paperWidthMm = m.paperWidthMm,
            )
        }

        // 2. Live mDNS network printers
        for (p in nsdDiscovered.values) {
            if (!byName.containsKey(p.name)) {
                byName[p.name] = p.copy(isDefault = snap.defaultPrinter == p.name)
            }
        }

        // 3. Paired Classic Bluetooth printers
        if (btTransport.hasConnectPermission()) {
            val adapter = context.getSystemService(BluetoothManager::class.java)?.adapter
            if (adapter != null && adapter.isEnabled) {
                for (dev in adapter.bondedDevices.orEmpty()) {
                    val major = dev.bluetoothClass?.majorDeviceClass ?: 0
                    val nameHint = (dev.name ?: "").lowercase()
                    val looksLikePrinter = major == BluetoothClass.Device.Major.IMAGING ||
                        nameHint.contains("print") || nameHint.contains("pos") ||
                        nameHint.contains("xp-") || nameHint.contains("zijiang") ||
                        nameHint.contains("epson") || nameHint.contains("zebra") ||
                        nameHint.contains("bixolon") || nameHint.contains("sunmi") ||
                        nameHint.contains("mpt") || nameHint.contains("rp")
                    if (!looksLikePrinter) continue
                    val label = dev.name?.takeIf { it.isNotBlank() } ?: "BT Printer ${dev.address}"
                    byName[label] = PrinterInfo(
                        name = label,
                        displayName = label,
                        transport = PrinterTransportType.BLUETOOTH_SPP,
                        address = "bt://${dev.address}",
                        protocol = PrinterProtocol.ESCPOS,
                        online = true,
                        status = "paired",
                        isDefault = snap.defaultPrinter == label,
                        dpi = 203,
                        paperWidthMm = 80,
                    )
                }
            }
        }

        // 4. Connected USB OTG Printer Class (0x07) devices
        val usbManager = context.getSystemService(Context.USB_SERVICE) as? UsbManager
        usbManager?.deviceList?.values?.forEach { dev ->
            if (!UsbOtgTransport.isPrinterDevice(dev)) return@forEach
            val baseLabel = dev.productName?.takeIf { it.isNotBlank() } ?: "USB Printer (${dev.vendorId}:${dev.productId})"
            var label = baseLabel
            var suffix = 2
            while (byName.containsKey(label)) {
                label = "$baseLabel #$suffix"
                suffix++
            }
            val hasPerm = usbManager.hasPermission(dev)
            byName[label] = PrinterInfo(
                name = label,
                displayName = label,
                transport = PrinterTransportType.USB_OTG,
                address = "usb://${UsbOtgTransport.formatKey(dev)}",
                protocol = PrinterProtocol.ESCPOS,
                online = hasPerm,
                status = if (hasPerm) "ready" else "permission_required",
                isDefault = snap.defaultPrinter == label,
                dpi = 203,
                paperWidthMm = 80,
            )
        }

        // 5. Android System Print fallback
        val sysName = "Android System Print"
        byName[sysName] = PrinterInfo(
            name = sysName,
            displayName = sysName,
            transport = PrinterTransportType.ANDROID_SYSTEM,
            address = "android://system-print",
            protocol = PrinterProtocol.PDF,
            online = true,
            status = "ready",
            isDefault = snap.defaultPrinter == sysName || byName.isEmpty(),
            dpi = 300,
            paperWidthMm = 210,
        )

        return byName.values.toList()
    }
}
