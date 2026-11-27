package com.company.odoonativeprint

import android.Manifest
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.hardware.usb.UsbManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.webkit.CookieManager
import android.webkit.ValueCallback
import android.webkit.WebStorage
import androidx.activity.ComponentActivity
import androidx.activity.OnBackPressedCallback
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.Crossfade
import androidx.compose.animation.core.tween
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.core.content.ContextCompat
import androidx.lifecycle.lifecycleScope
import com.company.odoonativeprint.transport.UsbOtgTransport
import com.company.odoonativeprint.ui.browser.BootScreen
import com.company.odoonativeprint.ui.browser.BrowserScreen
import com.company.odoonativeprint.ui.browser.TabManager
import com.company.odoonativeprint.ui.controlcenter.ControlCenterScreen
import com.company.odoonativeprint.ui.settings.SettingsScreen
import com.company.odoonativeprint.ui.theme.YaseirTheme

private enum class AppScreen { BOOT, BROWSER, CONTROL_CENTER, SETTINGS }

class MainActivity : ComponentActivity() {

    private val app: YaseirPrintApp
        get() = application as YaseirPrintApp

    private lateinit var tabManager: TabManager
    private var activeScreen by mutableStateOf(AppScreen.BOOT)
    private var pendingFileCallback: ValueCallback<Array<Uri>>? = null

    private val filePickerLauncher = registerForActivityResult(
        ActivityResultContracts.StartActivityForResult(),
    ) { result ->
        val uris = if (result.resultCode == RESULT_OK) {
            result.data?.data?.let { arrayOf(it) }
        } else {
            null
        }
        pendingFileCallback?.onReceiveValue(uris)
        pendingFileCallback = null
    }

    private val permissionLauncher = registerForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions(),
    ) {
        app.discovery.invalidate()
    }

    private val hardwareReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context?, intent: Intent?) {
            app.discovery.invalidate()
            if (intent?.action == UsbOtgTransport.ACTION_USB_PERMISSION) {
                app.queueEngine.printersWithQueuedJobs().forEach { app.worker.wakePrinter(it) }
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()

        tabManager = TabManager(
            context = this,
            dispatcher = app.bridgeDispatcher,
            bridgeScope = lifecycleScope,
            onShowFileChooser = { callback, params ->
                pendingFileCallback?.onReceiveValue(null)
                pendingFileCallback = callback
                runCatching {
                    filePickerLauncher.launch(params.createIntent())
                    true
                }.getOrElse {
                    pendingFileCallback = null
                    false
                }
            },
            onActiveUrlChanged = { url -> app.configStore.saveLastUrl(url) },
        )

        val initialOrigin = app.configStore.current().origin
        if (!initialOrigin.isNullOrBlank()) {
            tabManager.ensureInitialized(initialOrigin, app.configStore.resolveStartupUrl())
            activeScreen = AppScreen.BROWSER
        }

        requestRuntimePermissionsIfNeeded()
        registerHardwareReceivers()

        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() {
                when (activeScreen) {
                    AppScreen.CONTROL_CENTER, AppScreen.SETTINGS -> {
                        activeScreen = if (app.configStore.current().origin != null) AppScreen.BROWSER else AppScreen.BOOT
                    }
                    AppScreen.BROWSER -> {
                        // Keep the WebView and Odoo SPA session alive in background instead of destroying on Back.
                        if (!tabManager.goBack()) moveTaskToBack(true)
                    }
                    AppScreen.BOOT -> finish()
                }
            }
        })

        setContent {
            val config by app.configStore.state.collectAsState()
            val queueSummary by app.worker.summary.collectAsState()

            LaunchedEffect(Unit) {
                app.navEvents.collect { ev ->
                    when (ev) {
                        AppNavEvent.OpenControlCenter -> activeScreen = AppScreen.CONTROL_CENTER
                        AppNavEvent.OpenSettings -> activeScreen = AppScreen.SETTINGS
                        is AppNavEvent.OpenTab -> {
                            tabManager.openTab(ev.url)
                            activeScreen = AppScreen.BROWSER
                        }
                    }
                }
            }

            YaseirTheme(themeMode = config.themeMode, language = config.language) {
                Crossfade(
                    targetState = activeScreen,
                    animationSpec = tween(durationMillis = 180),
                    label = "screen_transition",
                ) { screen ->
                    when (screen) {
                        AppScreen.BOOT -> BootScreen(
                            initialOrigin = config.origin ?: "",
                            onOriginSaved = { normalized ->
                                app.configStore.saveOrigin(normalized, config.autoReprint)
                                tabManager.ensureInitialized(normalized)
                                activeScreen = AppScreen.BROWSER
                            },
                        )
                        AppScreen.BROWSER -> BrowserScreen(
                            configuredOrigin = config.origin ?: "",
                            tabManager = tabManager,
                            queueSummary = queueSummary,
                            onOpenControlCenter = { activeScreen = AppScreen.CONTROL_CENTER },
                            onOpenSettings = { activeScreen = AppScreen.SETTINGS },
                        )
                        AppScreen.CONTROL_CENTER -> ControlCenterScreen(
                            app = app,
                            onBack = { activeScreen = if (config.origin != null) AppScreen.BROWSER else AppScreen.BOOT },
                        )
                        AppScreen.SETTINGS -> SettingsScreen(
                            configStore = app.configStore,
                            onOriginUpdated = { normalized ->
                                tabManager.ensureInitialized(normalized)
                                activeScreen = AppScreen.BROWSER
                            },
                            onClearWebData = {
                                clearWebBrowsingData()
                                config.origin?.let { tabManager.ensureInitialized(it) }
                                activeScreen = AppScreen.BROWSER
                            },
                            onBack = { activeScreen = if (config.origin != null) AppScreen.BROWSER else AppScreen.BOOT },
                        )
                    }
                }
            }
        }
    }

    override fun onStart() {
        super.onStart()
        app.discovery.startMdnsDiscovery()
    }

    override fun onStop() {
        app.discovery.stopMdnsDiscovery()
        super.onStop()
    }

    override fun onDestroy() {
        runCatching { unregisterReceiver(hardwareReceiver) }
        tabManager.destroyAll()
        super.onDestroy()
    }

    private fun clearWebBrowsingData() {
        app.configStore.saveLastUrl(null)
        tabManager.destroyAll()
        CookieManager.getInstance().removeAllCookies(null)
        CookieManager.getInstance().flush()
        WebStorage.getInstance().deleteAllData()
    }

    private fun requestRuntimePermissionsIfNeeded() {
        val needed = mutableListOf<String>()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            needed += Manifest.permission.BLUETOOTH_CONNECT
            needed += Manifest.permission.BLUETOOTH_SCAN
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            needed += Manifest.permission.POST_NOTIFICATIONS
        }
        val missing = needed.filter {
            ContextCompat.checkSelfPermission(this, it) != PackageManager.PERMISSION_GRANTED
        }
        if (missing.isNotEmpty()) {
            permissionLauncher.launch(missing.toTypedArray())
        }
    }

    private fun registerHardwareReceivers() {
        val filter = IntentFilter().apply {
            addAction(UsbManager.ACTION_USB_DEVICE_ATTACHED)
            addAction(UsbManager.ACTION_USB_DEVICE_DETACHED)
            addAction(UsbOtgTransport.ACTION_USB_PERMISSION)
            addAction(BluetoothAdapter.ACTION_STATE_CHANGED)
            addAction(BluetoothDevice.ACTION_BOND_STATE_CHANGED)
        }
        ContextCompat.registerReceiver(this, hardwareReceiver, filter, ContextCompat.RECEIVER_NOT_EXPORTED)
    }
}
