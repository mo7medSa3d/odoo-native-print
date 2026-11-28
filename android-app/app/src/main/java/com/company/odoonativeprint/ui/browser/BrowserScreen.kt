package com.company.odoonativeprint.ui.browser
import com.company.odoonativeprint.ui.theme.tr

import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.automirrored.rounded.ArrowForward
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Fullscreen
import androidx.compose.material.icons.rounded.FullscreenExit
import androidx.compose.material.icons.rounded.Print
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import com.company.odoonativeprint.R
import com.company.odoonativeprint.core.QueueSummary
import com.company.odoonativeprint.ui.theme.LocalYaseirColors

/**
 * Multi-tab Odoo browser matching desktop app/dist/tabbar.html with POS fullscreen toggle.
 */
@Composable
fun BrowserScreen(
    configuredOrigin: String,
    tabManager: TabManager,
    queueSummary: QueueSummary,
    isVisible: Boolean = true,
    onOpenControlCenter: () -> Unit,
    onOpenSettings: () -> Unit,
) {
    val c = LocalYaseirColors.current
    val tabState by tabManager.state.collectAsState()
    val activeTab = tabState.activeTab
    var fullscreenPos by remember { mutableStateOf(false) }

    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(c.bgCanvas)
            .statusBarsPadding()
            .navigationBarsPadding()
            .imePadding(),
    ) {
        Column(modifier = Modifier.fillMaxSize()) {
            if (!fullscreenPos) {
                BoxWithConstraints(
                    modifier = Modifier
                        .fillMaxWidth()
                        .background(c.bgSurface)
                        .border(0.5.dp, c.border),
                ) {
                    val isCompact = maxWidth < 600.dp
                    Column(modifier = Modifier.fillMaxWidth()) {
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(horizontal = 8.dp, vertical = 6.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Image(
                                painter = painterResource(R.drawable.ic_yaseir_logo),
                                contentDescription = "Yaseir Print",
                                modifier = Modifier
                                    .size(28.dp)
                                    .clip(RoundedCornerShape(8.dp))
                                    .clickable(onClick = onOpenControlCenter),
                            )
                            Spacer(Modifier.width(4.dp))

                            IconButton(
                                onClick = { tabManager.goBack() },
                                enabled = activeTab?.canGoBack == true,
                                modifier = Modifier.size(32.dp),
                            ) {
                                Icon(
                                    Icons.AutoMirrored.Rounded.ArrowBack,
                                    contentDescription = tr("رجوع"),
                                    tint = if (activeTab?.canGoBack == true) c.textPrimary else c.textMuted.copy(alpha = 0.4f),
                                    modifier = Modifier.size(18.dp),
                                )
                            }
                            IconButton(
                                onClick = { tabManager.goForward() },
                                enabled = activeTab?.canGoForward == true,
                                modifier = Modifier.size(32.dp),
                            ) {
                                Icon(
                                    Icons.AutoMirrored.Rounded.ArrowForward,
                                    contentDescription = tr("تقدم"),
                                    tint = if (activeTab?.canGoForward == true) c.textPrimary else c.textMuted.copy(alpha = 0.4f),
                                    modifier = Modifier.size(18.dp),
                                )
                            }
                            IconButton(
                                onClick = { tabManager.reload() },
                                modifier = Modifier.size(32.dp),
                            ) {
                                Icon(
                                    Icons.Rounded.Refresh,
                                    contentDescription = tr("تحديث"),
                                    tint = c.textPrimary,
                                    modifier = Modifier.size(18.dp),
                                )
                            }

                            if (isCompact) {
                                Spacer(Modifier.weight(1f))
                            } else {
                                BrowserTabStrip(
                                    tabs = tabState.tabs,
                                    activeTabId = tabState.activeTabId,
                                    onSelect = { tabManager.switchTab(it) },
                                    onClose = { tabManager.closeTab(it) },
                                    onNewTab = { tabManager.openTab(configuredOrigin) },
                                    modifier = Modifier.weight(1f),
                                )
                            }

                            PrinterStatusPill(
                                queueSummary = queueSummary,
                                onClick = onOpenControlCenter,
                            )

                            IconButton(onClick = { fullscreenPos = true }, modifier = Modifier.size(32.dp)) {
                                Icon(Icons.Rounded.Fullscreen, contentDescription = tr("ملء الشاشة لنقطة البيع"), tint = c.textSecondary, modifier = Modifier.size(18.dp))
                            }
                            IconButton(onClick = onOpenSettings, modifier = Modifier.size(32.dp)) {
                                Icon(Icons.Rounded.Settings, contentDescription = tr("الإعدادات"), tint = c.textSecondary, modifier = Modifier.size(18.dp))
                            }
                        }

                        if (isCompact) {
                            BrowserTabStrip(
                                tabs = tabState.tabs,
                                activeTabId = tabState.activeTabId,
                                onSelect = { tabManager.switchTab(it) },
                                onClose = { tabManager.closeTab(it) },
                                onNewTab = { tabManager.openTab(configuredOrigin) },
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .padding(start = 8.dp, end = 8.dp, bottom = 6.dp),
                            )
                        }
                    }
                }

                if (activeTab != null && activeTab.progress in 1..99) {
                    LinearProgressIndicator(
                        progress = { activeTab.progress / 100f },
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(2.dp),
                        color = c.brand,
                        trackColor = c.bgSubtle,
                    )
                }
            }

            // Active WebView container (reattaches preserved WebView without reloading!)
            val currentWebView = tabManager.activeWebView()
            Box(modifier = Modifier.weight(1f).fillMaxWidth()) {
                if (currentWebView != null) {
                    AndroidView(
                        modifier = Modifier.fillMaxSize(),
                        factory = { ctx -> FrameLayout(ctx) },
                        update = { container ->
                            container.visibility = if (isVisible) View.VISIBLE else View.INVISIBLE
                            if (container.childCount == 1 && container.getChildAt(0) === currentWebView) return@AndroidView
                            container.removeAllViews()
                            (currentWebView.parent as? ViewGroup)?.removeView(currentWebView)
                            container.addView(
                                currentWebView,
                                FrameLayout.LayoutParams(
                                    ViewGroup.LayoutParams.MATCH_PARENT,
                                    ViewGroup.LayoutParams.MATCH_PARENT,
                                ),
                            )
                        },
                    )
                }
            }
        }

        // Floating pill when in Fullscreen POS mode
        if (fullscreenPos) {
            Row(
                modifier = Modifier
                    .align(Alignment.TopEnd)
                    .padding(10.dp)
                    .clip(RoundedCornerShape(999.dp))
                    .background(c.bgSurface.copy(alpha = 0.92f))
                    .border(1.dp, c.border, RoundedCornerShape(999.dp))
                    .padding(horizontal = 8.dp, vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                IconButton(onClick = onOpenControlCenter, modifier = Modifier.size(30.dp)) {
                    Icon(Icons.Rounded.Print, contentDescription = tr("مركز التحكم"), tint = c.brand, modifier = Modifier.size(18.dp))
                }
                IconButton(onClick = { fullscreenPos = false }, modifier = Modifier.size(30.dp)) {
                    Icon(Icons.Rounded.FullscreenExit, contentDescription = tr("إظهار الشريط"), tint = c.textPrimary, modifier = Modifier.size(18.dp))
                }
            }
        }
    }
}

@Composable
private fun BrowserTabStrip(
    tabs: List<BrowserTab>,
    activeTabId: String?,
    onSelect: (String) -> Unit,
    onClose: (String) -> Unit,
    onNewTab: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val c = LocalYaseirColors.current
    Row(
        modifier = modifier
            .horizontalScroll(rememberScrollState())
            .padding(horizontal = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        tabs.forEach { tab ->
            val selected = tab.id == activeTabId
            Row(
                modifier = Modifier
                    .widthIn(min = 72.dp, max = 164.dp)
                    .clip(RoundedCornerShape(999.dp))
                    .background(if (selected) c.bgHover else c.bgSubtle)
                    .border(
                        width = 1.dp,
                        color = if (selected) c.brand else c.border,
                        shape = RoundedCornerShape(999.dp),
                    )
                    .clickable { onSelect(tab.id) }
                    .padding(horizontal = 10.dp, vertical = 5.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = tab.title.ifBlank { "Odoo" },
                    style = MaterialTheme.typography.labelSmall,
                    color = if (selected) c.textPrimary else c.textSecondary,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f, fill = false),
                )
                if (tabs.size > 1) {
                    Spacer(Modifier.width(4.dp))
                    Icon(
                        Icons.Rounded.Close,
                        contentDescription = tr("إغلاق التبويب"),
                        tint = c.textMuted,
                        modifier = Modifier
                            .size(14.dp)
                            .clickable { onClose(tab.id) },
                    )
                }
            }
        }

        IconButton(
            onClick = onNewTab,
            modifier = Modifier.size(28.dp),
        ) {
            Icon(Icons.Rounded.Add, contentDescription = tr("تبويب جديد"), tint = c.textSecondary, modifier = Modifier.size(18.dp))
        }
    }
}

@Composable
private fun PrinterStatusPill(
    queueSummary: QueueSummary,
    onClick: () -> Unit,
) {
    val c = LocalYaseirColors.current
    val dotColor = when {
        queueSummary.failed > 0 -> c.errFg
        queueSummary.active > 0 -> c.warnFg
        else -> c.okFg
    }
    Row(
        modifier = Modifier
            .clip(RoundedCornerShape(999.dp))
            .background(c.bgSubtle)
            .border(1.dp, c.border, RoundedCornerShape(999.dp))
            .clickable(onClick = onClick)
            .padding(horizontal = 10.dp, vertical = 5.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(5.dp),
    ) {
        Box(
            modifier = Modifier
                .size(8.dp)
                .clip(CircleShape)
                .background(dotColor),
        )
        Icon(Icons.Rounded.Print, contentDescription = tr("مركز التحكم"), tint = c.textPrimary, modifier = Modifier.size(16.dp))
        if (queueSummary.active > 0) {
            Text(
                text = "${queueSummary.active}",
                style = MaterialTheme.typography.labelSmall,
                color = c.brandText,
                maxLines = 1,
            )
        }
    }
}
