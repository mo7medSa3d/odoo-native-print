package com.company.odoonativeprint.ui.browser

import android.content.Context
import android.net.Uri
import android.view.ViewGroup
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebView
import com.company.odoonativeprint.bridge.BridgeDispatcher
import com.company.odoonativeprint.bridge.OdooWebViewFactory
import com.company.odoonativeprint.bridge.WebViewCallbacks
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.util.UUID

data class BrowserTab(
    val id: String,
    val title: String,
    val url: String,
    val progress: Int = 100,
    val canGoBack: Boolean = false,
    val canGoForward: Boolean = false,
)

data class TabManagerState(
    val tabs: List<BrowserTab> = emptyList(),
    val activeTabId: String? = null,
) {
    val activeTab: BrowserTab?
        get() = tabs.firstOrNull { it.id == activeTabId } ?: tabs.firstOrNull()
}

/**
 * Manages live Android WebView instances across tabs so switching tabs or rotating
 * a POS tablet never reloads active Odoo sessions.
 */
class TabManager(
    private val context: Context,
    private val dispatcher: BridgeDispatcher,
    private val bridgeScope: CoroutineScope,
    private val onShowFileChooser: (ValueCallback<Array<Uri>>, WebChromeClient.FileChooserParams) -> Boolean,
    private val onActiveUrlChanged: ((String) -> Unit)? = null,
) {
    private val webViews = LinkedHashMap<String, WebView>()
    private val _state = MutableStateFlow(TabManagerState())
    val state: StateFlow<TabManagerState> = _state.asStateFlow()

    private var activeOrigin: String? = null

    fun ensureInitialized(origin: String, initialUrl: String? = null) {
        if (activeOrigin != origin) {
            destroyAll()
            activeOrigin = origin
        }
        if (_state.value.tabs.isEmpty()) {
            openTab(initialUrl?.takeIf { it.isNotBlank() } ?: origin)
        }
    }

    fun openTab(url: String): String {
        val origin = activeOrigin ?: return ""
        if (_state.value.tabs.size >= MAX_TABS) {
            // Focus the last tab and navigate it instead of exhausting tablet RAM
            val reuseId = _state.value.tabs.last().id
            webViews[reuseId]?.loadUrl(url)
            switchTab(reuseId)
            return reuseId
        }

        val id = UUID.randomUUID().toString()
        val wv = OdooWebViewFactory.create(
            context = context,
            configuredOrigin = origin,
            dispatcher = dispatcher,
            bridgeScope = bridgeScope,
            callbacks = WebViewCallbacks(
                onTitleChanged = { title -> updateTab(id) { it.copy(title = title) } },
                onUrlChanged = { newUrl ->
                    updateTab(id) { it.copy(url = newUrl) }
                    if (_state.value.activeTabId == id) {
                        onActiveUrlChanged?.invoke(newUrl)
                    }
                },
                onProgressChanged = { p -> updateTab(id) { it.copy(progress = p) } },
                onNavigationStateChanged = { back, fwd ->
                    updateTab(id) { it.copy(canGoBack = back, canGoForward = fwd) }
                },
                onOpenNewTab = { newUrl -> openTab(newUrl) },
                onShowFileChooser = onShowFileChooser,
            ),
        )
        webViews[id] = wv
        val tab = BrowserTab(id = id, title = "Odoo", url = url, progress = 10)
        _state.value = TabManagerState(
            tabs = _state.value.tabs + tab,
            activeTabId = id,
        )
        wv.loadUrl(url)
        return id
    }

    fun switchTab(id: String) {
        val target = _state.value.tabs.firstOrNull { it.id == id } ?: return
        _state.value = _state.value.copy(activeTabId = id)
        if (target.url.isNotBlank()) {
            onActiveUrlChanged?.invoke(target.url)
        }
    }

    fun closeTab(id: String) {
        val current = _state.value.tabs
        val idx = current.indexOfFirst { it.id == id }
        if (idx == -1) return
        webViews.remove(id)?.apply {
            (parent as? ViewGroup)?.removeView(this)
            stopLoading()
            destroy()
        }
        val remaining = current.filterNot { it.id == id }
        if (remaining.isEmpty()) {
            _state.value = TabManagerState()
            activeOrigin?.let { openTab(it) }
            return
        }
        val nextActive = if (_state.value.activeTabId == id) {
            remaining[idx.coerceAtMost(remaining.lastIndex)].id
        } else {
            _state.value.activeTabId
        }
        _state.value = TabManagerState(tabs = remaining, activeTabId = nextActive)
    }

    fun activeWebView(): WebView? = _state.value.activeTabId?.let { webViews[it] }

    fun goBack(): Boolean {
        val wv = activeWebView() ?: return false
        if (wv.canGoBack()) {
            wv.goBack()
            return true
        }
        val state = _state.value
        if (state.tabs.size > 1 && state.activeTabId != null) {
            closeTab(state.activeTabId)
            return true
        }
        return false
    }

    fun goForward() { activeWebView()?.takeIf { it.canGoForward() }?.goForward() }
    fun reload() { activeWebView()?.reload() }

    fun destroyAll() {
        webViews.values.forEach { wv ->
            runCatching {
                (wv.parent as? ViewGroup)?.removeView(wv)
                wv.stopLoading()
                wv.destroy()
            }
        }
        webViews.clear()
        _state.value = TabManagerState()
    }

    private fun updateTab(id: String, transform: (BrowserTab) -> BrowserTab) {
        val updated = _state.value.tabs.map { if (it.id == id) transform(it) else it }
        _state.value = _state.value.copy(tabs = updated)
    }

    companion object {
        private const val MAX_TABS = 8
    }
}
