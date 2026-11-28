package com.company.odoonativeprint.bridge

import android.Manifest
import android.annotation.SuppressLint
import android.app.DownloadManager
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.os.Message
import android.provider.MediaStore
import android.util.Base64
import android.webkit.CookieManager
import android.webkit.PermissionRequest
import android.webkit.URLUtil
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.Toast
import androidx.core.content.ContextCompat
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import com.company.odoonativeprint.core.OriginPolicy
import kotlinx.coroutines.CoroutineScope
import java.io.File

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

    internal enum class ExternalSchemeAction { ALLOW_WEBVIEW, LAUNCH_EXTERNAL, BLOCK }

    internal fun classifyNavigationScheme(scheme: String?, hasGesture: Boolean): ExternalSchemeAction {
        val lower = scheme?.lowercase() ?: return ExternalSchemeAction.BLOCK
        return when (lower) {
            "http", "https" -> ExternalSchemeAction.ALLOW_WEBVIEW
            "mailto", "tel" -> if (hasGesture) ExternalSchemeAction.LAUNCH_EXTERNAL else ExternalSchemeAction.BLOCK
            else -> ExternalSchemeAction.BLOCK
        }
    }

    internal fun isAllowedPopupUrl(url: String?, isUserGesture: Boolean): Boolean {
        if (!isUserGesture || url.isNullOrBlank()) return false
        val lower = url.trim().lowercase()
        return lower.startsWith("http://") || lower.startsWith("https://")
    }

    @Volatile
    private var cachedBridgeJs: String? = null

    private fun loadBridgeJs(context: Context): String =
        cachedBridgeJs ?: context.assets.open("bridge.js").bufferedReader(Charsets.UTF_8).use { it.readText() }
            .also { cachedBridgeJs = it }

    @SuppressLint("SetJavaScriptEnabled")
    fun create(
        context: Context,
        configuredOrigin: String,
        dispatcher: BridgeDispatcher,
        bridgeScope: CoroutineScope,
        callbacks: WebViewCallbacks,
    ): WebView {
        val bridgeJs = loadBridgeJs(context)
        val allowedOrigins = setOf(configuredOrigin)

        return WebView(context).apply {
            settings.apply {
                javaScriptEnabled = true
                domStorageEnabled = true
                databaseEnabled = true
                javaScriptCanOpenWindowsAutomatically = false
                setSupportMultipleWindows(true)
                // Respect viewport meta tags and scale down wide content on narrow phones
                // instead of forcing 100% physical-pixel scale and clipping horizontally.
                useWideViewPort = true
                loadWithOverviewMode = true
                layoutAlgorithm = WebSettings.LayoutAlgorithm.TEXT_AUTOSIZING
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

                override fun doUpdateVisitedHistory(view: WebView, url: String?, isReload: Boolean) {
                    super.doUpdateVisitedHistory(view, url, isReload)
                    if (!url.isNullOrBlank()) callbacks.onUrlChanged(url)
                    callbacks.onNavigationStateChanged(view.canGoBack(), view.canGoForward())
                }

                override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                    val uri = request.url ?: return true
                    return when (classifyNavigationScheme(uri.scheme, request.hasGesture())) {
                        ExternalSchemeAction.ALLOW_WEBVIEW -> false
                        ExternalSchemeAction.LAUNCH_EXTERNAL -> {
                            runCatching {
                                context.startActivity(
                                    Intent(Intent.ACTION_VIEW, uri).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                                )
                            }
                            true
                        }
                        ExternalSchemeAction.BLOCK -> true
                    }
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
                    if (!isUserGesture) return false
                    val href = view?.hitTestResult?.extra
                    if (!href.isNullOrBlank()) {
                        routePopupUrl(context, configuredOrigin, href, isUserGesture, callbacks)
                        return false
                    }
                    val transport = resultMsg?.obj as? WebView.WebViewTransport ?: return false
                    val temp = WebView(context)
                    temp.postDelayed({ runCatching { temp.destroy() } }, 10_000L)
                    temp.webViewClient = object : WebViewClient() {
                        override fun shouldOverrideUrlLoading(v: WebView, request: WebResourceRequest): Boolean {
                            val url = request.url?.toString()
                            if (!url.isNullOrBlank()) {
                                routePopupUrl(context, configuredOrigin, url, isUserGesture, callbacks)
                            }
                            v.destroy()
                            return true
                        }
                    }
                    transport.webView = temp
                    resultMsg.sendToTarget()
                    return true
                }

                override fun onPermissionRequest(request: PermissionRequest) {
                    val reqOrigin = request.origin?.toString() ?: ""
                    val hasCamera = ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
                        PackageManager.PERMISSION_GRANTED
                    if (hasCamera &&
                        OriginPolicy.isSameOrigin(configuredOrigin, reqOrigin) &&
                        PermissionRequest.RESOURCE_VIDEO_CAPTURE in request.resources
                    ) {
                        request.grant(arrayOf(PermissionRequest.RESOURCE_VIDEO_CAPTURE))
                    } else {
                        request.deny()
                    }
                }

                override fun onShowFileChooser(
                    webView: WebView?,
                    filePathCallback: ValueCallback<Array<Uri>>,
                    fileChooserParams: FileChooserParams,
                ): Boolean = callbacks.onShowFileChooser(filePathCallback, fileChooserParams)
            }

            setDownloadListener { url, userAgent, contentDisposition, mimetype, _ ->
                handleWebDownload(context, configuredOrigin, url, userAgent, contentDisposition, mimetype)
            }
        }
    }

    private fun handleWebDownload(
        context: Context,
        configuredOrigin: String,
        url: String?,
        userAgent: String?,
        contentDisposition: String?,
        mimetype: String?,
    ) {
        if (url.isNullOrBlank()) return
        val fileName = URLUtil.guessFileName(url, contentDisposition, mimetype)
            .ifBlank { "odoo-download" }
            .replace(Regex("[/\\\\:*?\"<>|]"), "_")
        if (url.startsWith("data:", ignoreCase = true)) {
            saveDataUrlDownload(context, url, fileName, mimetype ?: "application/octet-stream")
            return
        }
        if (!OriginPolicy.isSameOrigin(configuredOrigin, url) &&
            !url.startsWith("http://", ignoreCase = true) &&
            !url.startsWith("https://", ignoreCase = true)
        ) {
            return
        }
        runCatching {
            val req = DownloadManager.Request(Uri.parse(url)).apply {
                setMimeType(mimetype)
                CookieManager.getInstance().getCookie(url)?.takeIf { it.isNotBlank() }?.let {
                    addRequestHeader("Cookie", it)
                }
                userAgent?.takeIf { it.isNotBlank() }?.let { addRequestHeader("User-Agent", it) }
                setTitle(fileName)
                setNotificationVisibility(DownloadManager.Request.VISIBILITY_VISIBLE_NOTIFY_COMPLETED)
                setDestinationInExternalPublicDir(Environment.DIRECTORY_DOWNLOADS, fileName)
            }
            val dm = context.getSystemService(Context.DOWNLOAD_SERVICE) as? DownloadManager
            dm?.enqueue(req)
            Toast.makeText(context, "Downloading $fileName", Toast.LENGTH_SHORT).show()
        }
    }

    private fun saveDataUrlDownload(context: Context, dataUrl: String, fileName: String, mime: String) {
        val comma = dataUrl.indexOf(',')
        if (comma <= 5) return
        val header = dataUrl.substring(5, comma).lowercase()
        val body = dataUrl.substring(comma + 1)
        val bytes = runCatching {
            if (header.contains(";base64")) Base64.decode(body, Base64.DEFAULT) else Uri.decode(body).toByteArray(Charsets.UTF_8)
        }.getOrNull() ?: return
        runCatching {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                val values = ContentValues().apply {
                    put(MediaStore.Downloads.DISPLAY_NAME, fileName)
                    put(MediaStore.Downloads.MIME_TYPE, mime)
                    put(MediaStore.Downloads.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS)
                }
                val resolver = context.contentResolver
                val item = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values) ?: return
                resolver.openOutputStream(item)?.use { it.write(bytes) }
            } else {
                val dir = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS)
                dir.mkdirs()
                File(dir, fileName).writeBytes(bytes)
            }
            Toast.makeText(context, "Saved $fileName", Toast.LENGTH_SHORT).show()
        }
    }

    private fun routePopupUrl(
        context: Context,
        configuredOrigin: String,
        url: String,
        isUserGesture: Boolean,
        callbacks: WebViewCallbacks,
    ) {
        if (!isAllowedPopupUrl(url, isUserGesture)) return
        if (OriginPolicy.isSameOrigin(configuredOrigin, url)) {
            callbacks.onOpenNewTab(url)
        } else {
            runCatching {
                context.startActivity(
                    Intent(Intent.ACTION_VIEW, Uri.parse(url)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                )
            }
        }
    }
}
