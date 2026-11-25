package com.company.odoonativeprint.bridge

import android.annotation.SuppressLint
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.net.Uri
import android.os.Message
import android.webkit.CookieManager
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import com.company.odoonativeprint.core.OriginPolicy
import kotlinx.coroutines.CoroutineScope

data class WebViewCallbacks(
    val onTitleChanged: (String) -> Unit,
    val onUrlChanged: (String) -> Unit,
    val onProgressChanged: (Int) -> Unit,
    val onNavigationStateChanged: (canGoBack: Boolean, canGoForward: Boolean) -> Unit,
    val onOpenNewTab: (String) -> Unit,
    val onShowFileChooser: (ValueCallback<Array<Uri>>, WebChromeClient.FileChooserParams) -> Boolean,
)

/**
 * Creates hardened Odoo-ready Android WebViews with origin-scoped WebMessageListener
 * and DOCUMENT_START_SCRIPT injection of bridge.js.
 */
object OdooWebViewFactory {

    @SuppressLint("SetJavaScriptEnabled")
    fun create(
        context: Context,
        configuredOrigin: String,
        dispatcher: BridgeDispatcher,
        bridgeScope: CoroutineScope,
        callbacks: WebViewCallbacks,
    ): WebView {
        val bridgeJs = context.assets.open("bridge.js").bufferedReader(Charsets.UTF_8).use { it.readText() }
        val allowedOrigins = setOf(configuredOrigin)

        return WebView(context).apply {
            setInitialScale(100)
            settings.apply {
                javaScriptEnabled = true
                domStorageEnabled = true
                databaseEnabled = true
                javaScriptCanOpenWindowsAutomatically = true
                setSupportMultipleWindows(true)
                // Render Odoo at device width. Overview mode shrinks pages that
                // have no viewport meta tag and hides controls behind the
                // browser chrome, which makes POS/print screens feel cut off.
                useWideViewPort = true
                loadWithOverviewMode = false
                builtInZoomControls = true
                displayZoomControls = false
                mediaPlaybackRequiresUserGesture = false
                allowFileAccess = false
                allowContentAccess = true
                mixedContentMode = WebSettings.MIXED_CONTENT_COMPATIBILITY_MODE
            }

            val cookieMgr = CookieManager.getInstance()
            cookieMgr.setAcceptCookie(true)
            cookieMgr.setAcceptThirdPartyCookies(this, true)

            if (WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER)) {
                WebViewCompat.addWebMessageListener(
                    this,
                    "__NP_ANDROID_PORT__",
                    allowedOrigins,
                    AndroidWebBridgeListener(dispatcher, bridgeScope),
                )
            }

            val hasDocStart = WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)
            if (hasDocStart) {
                WebViewCompat.addDocumentStartJavaScript(this, bridgeJs, allowedOrigins)
            }

            webViewClient = object : WebViewClient() {
                override fun onPageStarted(view: WebView, url: String?, favicon: Bitmap?) {
                    super.onPageStarted(view, url, favicon)
                    if (!url.isNullOrBlank()) {
                        callbacks.onUrlChanged(url)
                        if (!hasDocStart && OriginPolicy.isSameOrigin(configuredOrigin, url)) {
                            view.evaluateJavascript(bridgeJs, null)
                        }
                    }
                    callbacks.onNavigationStateChanged(view.canGoBack(), view.canGoForward())
                }

                override fun onPageFinished(view: WebView, url: String?) {
                    super.onPageFinished(view, url)
                    if (!url.isNullOrBlank()) callbacks.onUrlChanged(url)
                    callbacks.onNavigationStateChanged(view.canGoBack(), view.canGoForward())
                }

                override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                    val uri = request.url ?: return false
                    val scheme = uri.scheme?.lowercase() ?: return false
                    if (scheme != "http" && scheme != "https") {
                        runCatching {
                            context.startActivity(Intent(Intent.ACTION_VIEW, uri).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                        }
                        return true
                    }
                    return false
                }
            }

            webChromeClient = object : WebChromeClient() {
                override fun onReceivedTitle(view: WebView?, title: String?) {
                    if (!title.isNullOrBlank()) callbacks.onTitleChanged(title)
                }

                override fun onProgressChanged(view: WebView?, newProgress: Int) {
                    callbacks.onProgressChanged(newProgress)
                }

                override fun onCreateWindow(
                    view: WebView?,
                    isDialog: Boolean,
                    isUserGesture: Boolean,
                    resultMsg: Message?,
                ): Boolean {
                    val href = view?.hitTestResult?.extra
                    if (!href.isNullOrBlank()) {
                        if (OriginPolicy.isSameOrigin(configuredOrigin, href)) {
                            callbacks.onOpenNewTab(href)
                        } else {
                            runCatching {
                                context.startActivity(
                                    Intent(Intent.ACTION_VIEW, Uri.parse(href)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                                )
                            }
                        }
                    }
                    return false
                }

                override fun onShowFileChooser(
                    webView: WebView?,
                    filePathCallback: ValueCallback<Array<Uri>>,
                    fileChooserParams: FileChooserParams,
                ): Boolean = callbacks.onShowFileChooser(filePathCallback, fileChooserParams)
            }
        }
    }
}
