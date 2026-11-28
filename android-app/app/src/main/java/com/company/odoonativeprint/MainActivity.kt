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
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
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
                // Keep BrowserScreen mounted with WebView hidden instead of Crossfade-detaching it;
                // fading or detaching hardware WebView surfaces caused the navigation stutter.
                val hasOrigin = config.origin != null
                Box(modifier = Modifier.fillMaxSize()) {
                    if (hasOrigin || activeScreen == AppScreen.BROWSER) {
                        BrowserScreen(
                            configuredOrigin = config.origin ?: "",
                            tabManager = tabManager,
                            queueSummary = queueSummary,
                            isVisible = activeScreen == AppScreen.BROWSER,
                            onOpenControlCenter = { activeScreen = AppScreen.CONTROL_CENTER },
                            onOpenSettings = { activeScreen = AppScreen.SETTINGS },
                        )
                    }
                    when (activeScreen) {
                        AppScreen.BOOT -> BootScreen(
                            initialOrigin = config.origin ?: "",
                            onOriginSaved = { normalized ->
                                app.configStore.saveOrigin(normalized, config.autoReprint)
                                tabManager.ensureInitialized(normalized)
                                activeScreen = AppScreen.BROWSER
                            },
                        )
                        AppScreen.CONTROL_CENTER -> Box(
                            modifier = Modifier
                                .fillMaxSize()
                                .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) {},
                        ) {
                            ControlCenterScreen(
                                app = app,
                                onBack = { activeScreen = if (hasOrigin) AppScreen.BROWSER else AppScreen.BOOT },
                            )
                        }
                        AppScreen.SETTINGS -> Box(
                            modifier = Modifier
                                .fillMaxSize()
                                .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) {},
                        ) {
                            SettingsScreen(
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
                                onBack = { activeScreen = if (hasOrigin) AppScreen.BROWSER else AppScreen.BOOT },
                            )
                        }
                        AppScreen.BROWSER -> Unit
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
