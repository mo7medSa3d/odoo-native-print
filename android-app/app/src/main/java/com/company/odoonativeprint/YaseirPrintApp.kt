package com.company.odoonativeprint

import android.app.Activity
import android.app.Application
import android.os.Bundle
import java.lang.ref.WeakReference
import com.company.odoonativeprint.bridge.BridgeDispatcher
import com.company.odoonativeprint.bridge.RemoteBridgeHandler
import com.company.odoonativeprint.core.AppConfigStore
import com.company.odoonativeprint.core.AppOverview
import com.company.odoonativeprint.core.JobQueueEngine
import com.company.odoonativeprint.core.PrintJobRecord
import com.company.odoonativeprint.core.PrinterCapabilities
import com.company.odoonativeprint.core.PrinterInfo
import com.company.odoonativeprint.core.SqliteJobPersistence
import com.company.odoonativeprint.transport.BluetoothSppTransport
import com.company.odoonativeprint.transport.IppTransport
import com.company.odoonativeprint.transport.PrinterDiscoveryService
import com.company.odoonativeprint.transport.Tcp9100Transport
import com.company.odoonativeprint.transport.UsbOtgTransport
import com.company.odoonativeprint.worker.AndroidJobExecutor
import com.company.odoonativeprint.worker.PrintForegroundService
import com.company.odoonativeprint.worker.PrintWorkerCoordinator
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import org.json.JSONObject
import java.io.File

sealed interface AppNavEvent {
    data object OpenControlCenter : AppNavEvent
    data object OpenSettings : AppNavEvent
    data class OpenTab(val url: String) : AppNavEvent
}

/**
 * Lightweight dependency container (zero DI framework ceremony).
 */
class YaseirPrintApp : Application() {

    lateinit var configStore: AppConfigStore
        private set
    lateinit var queueEngine: JobQueueEngine
        private set
    lateinit var discovery: PrinterDiscoveryService
        private set
    lateinit var worker: PrintWorkerCoordinator
        private set
    lateinit var bridgeDispatcher: BridgeDispatcher
        private set

    private val _navEvents = MutableSharedFlow<AppNavEvent>(extraBufferCapacity = 16)
    val navEvents = _navEvents.asSharedFlow()

    @Volatile
    private var currentActivityRef: WeakReference<Activity>? = null

    override fun onCreate() {
        super.onCreate()
        registerActivityLifecycleCallbacks(object : ActivityLifecycleCallbacks {
            override fun onActivityResumed(activity: Activity) { currentActivityRef = WeakReference(activity) }
            override fun onActivityPaused(activity: Activity) {
                if (currentActivityRef?.get() === activity) currentActivityRef = null
            }
            override fun onActivityCreated(a: Activity, b: Bundle?) = Unit
            override fun onActivityStarted(a: Activity) = Unit
            override fun onActivityStopped(a: Activity) = Unit
            override fun onActivitySaveInstanceState(a: Activity, b: Bundle) = Unit
            override fun onActivityDestroyed(a: Activity) = Unit
        })
        configStore = AppConfigStore(this)
        val persistence = SqliteJobPersistence(this)
        queueEngine = JobQueueEngine(
            persistence = persistence,
            spoolDir = File(filesDir, "spool"),
        )
        val tcp9100 = Tcp9100Transport()
        val ipp = IppTransport()
        val bt = BluetoothSppTransport(this)
        val usb = UsbOtgTransport(this)

        discovery = PrinterDiscoveryService(this, configStore, tcp9100, bt)
        val executor = AndroidJobExecutor(this, configStore, discovery, tcp9100, ipp, bt, usb) {
            currentActivityRef?.get()?.takeUnless { it.isFinishing || it.isDestroyed }
        }
        worker = PrintWorkerCoordinator(
            queue = queueEngine,
            executor = executor,
            onActiveCountChanged = { active -> PrintForegroundService.sync(this, active) },
        )
        worker.start(autoReprint = configStore.current().autoReprint)
        discovery.startMdnsDiscovery()

        bridgeDispatcher = BridgeDispatcher(object : RemoteBridgeHandler {
            override fun configuredOrigin(): String? = configStore.current().origin

            override fun enqueuePdf(
                printer: String,
                bytes: ByteArray,
                copies: Int,
                idempotencyKey: String?,
                options: JSONObject,
            ): PrintJobRecord = queueEngine.enqueue(
                printerName = resolvePrinterName(printer),
                sourceKind = "pdf",
                payloadBytes = bytes,
                copies = copies,
                idempotencyKey = idempotencyKey,
                optionsJson = options.toString(),
            ).also { worker.wakePrinter(it.printerName) }

            override fun enqueueText(
                printer: String,
                text: String,
                copies: Int,
                idempotencyKey: String?,
                options: JSONObject,
            ): PrintJobRecord = queueEngine.enqueue(
                printerName = resolvePrinterName(printer),
                sourceKind = "text",
                payloadBytes = text.toByteArray(Charsets.UTF_8),
                copies = copies,
                idempotencyKey = idempotencyKey,
                optionsJson = options.toString(),
            ).also { worker.wakePrinter(it.printerName) }

            override fun enqueueImage(
                printer: String,
                bytes: ByteArray,
                copies: Int,
                idempotencyKey: String?,
                options: JSONObject,
            ): PrintJobRecord = queueEngine.enqueue(
                printerName = resolvePrinterName(printer),
                sourceKind = "image",
                payloadBytes = bytes,
                copies = copies,
                idempotencyKey = idempotencyKey,
                optionsJson = options.toString(),
            ).also { worker.wakePrinter(it.printerName) }

            override fun printStatus(jobId: String): String? = queueEngine.getJob(jobId)?.state?.wireName
            override fun jobDetail(jobId: String): PrintJobRecord? = queueEngine.getJob(jobId)

            override fun appOverview(force: Boolean): AppOverview {
                val printers = discovery.listPrinters(force)
                return AppOverview(
                    version = BuildConfig.VERSION_NAME,
                    platform = "android",
                    configuredOrigin = configStore.current().origin,
                    printers = printers,
                    queue = queueEngine.summary(),
                    capsEpoch = discovery.capsEpoch(),
                    defaultPrinter = configStore.current().defaultPrinter
                        ?: printers.firstOrNull { it.isDefault }?.name,
                )
            }

            override fun listPrinters(force: Boolean): List<PrinterInfo> = discovery.listPrinters(force)
            override fun printerForms(printer: String): List<String> =
                discovery.capabilitiesFor(printer)?.forms ?: listOf("80mm Roll", "A4")
            override fun printerCapabilities(printer: String): PrinterCapabilities? =
                discovery.capabilitiesFor(printer)

            override fun openControlCenter() { _navEvents.tryEmit(AppNavEvent.OpenControlCenter) }
            override fun openSettings() { _navEvents.tryEmit(AppNavEvent.OpenSettings) }
            override fun openTab(url: String) { _navEvents.tryEmit(AppNavEvent.OpenTab(url)) }
            override fun reportEnvironment(report: String) { configStore.lastEnvironmentReport = report }
        })
    }

    fun enqueueTestPrint(printerName: String): PrintJobRecord {
        val job = queueEngine.enqueue(
            printerName = printerName,
            sourceKind = "test",
            payloadBytes = "TEST".toByteArray(Charsets.UTF_8),
            copies = 1,
            maxAttempts = 1,
        )
        worker.wakePrinter(job.printerName)
        return job
    }

    private fun resolvePrinterName(requested: String): String {
        val clean = requested.trim()
        if (clean.isNotEmpty()) return clean
        return configStore.current().defaultPrinter
            ?: discovery.listPrinters(false).firstOrNull()?.name
            ?: "Android System Print"
    }
}
