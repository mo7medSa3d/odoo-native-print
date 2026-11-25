package com.company.odoonativeprint.ui.controlcenter

import androidx.compose.animation.animateColorAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
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
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.DeleteSweep
import androidx.compose.material.icons.rounded.Print
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Share
import androidx.compose.material.icons.rounded.Tune
import androidx.compose.material.icons.rounded.Usb
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.company.odoonativeprint.BuildConfig
import com.company.odoonativeprint.YaseirPrintApp
import com.company.odoonativeprint.core.DiagnosticsExporter
import com.company.odoonativeprint.core.JobState
import com.company.odoonativeprint.core.PrintJobRecord
import com.company.odoonativeprint.core.PrinterInfo
import com.company.odoonativeprint.core.PrinterTransportType
import com.company.odoonativeprint.transport.UsbOtgTransport
import com.company.odoonativeprint.ui.theme.LocalYaseirColors
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Minimal iOS-style SaaS Control Center (Printers, Queue, Diagnostics).
 */
@Composable
fun ControlCenterScreen(
    app: YaseirPrintApp,
    onBack: () -> Unit,
) {
    val c = LocalYaseirColors.current
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    val liveSummary by app.worker.summary.collectAsState()

    var selectedTab by remember { mutableIntStateOf(0) }
    var printers by remember { mutableStateOf<List<PrinterInfo>>(emptyList()) }
    var jobs by remember { mutableStateOf<List<PrintJobRecord>>(emptyList()) }
    var editingPrinter by remember { mutableStateOf<PrinterInfo?>(null) }
    var showAddPrinter by remember { mutableStateOf(false) }

    fun reloadData(forcePrinters: Boolean = false) {
        scope.launch {
            val (p, j) = withContext(Dispatchers.IO) {
                app.discovery.listPrinters(force = forcePrinters) to app.queueEngine.listJobs(100)
            }
            printers = p
            jobs = j
        }
    }

    LaunchedEffect(Unit) { reloadData(forcePrinters = false) }
    LaunchedEffect(liveSummary) {
        jobs = withContext(Dispatchers.IO) { app.queueEngine.listJobs(100) }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(c.bgCanvas)
            .statusBarsPadding()
            .navigationBarsPadding()
            .imePadding(),
    ) {
        // Minimal translucent-feel top header
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .background(c.bgSurface)
                .padding(horizontal = 12.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = onBack) {
                Icon(Icons.AutoMirrored.Rounded.ArrowBack, contentDescription = "رجوع", tint = c.textPrimary)
            }
            Text(
                "مركز التحكم",
                style = MaterialTheme.typography.titleMedium,
                color = c.textPrimary,
                modifier = Modifier.weight(1f),
            )
            IconButton(onClick = { reloadData(forcePrinters = true) }) {
                Icon(Icons.Rounded.Refresh, contentDescription = "تحديث", tint = c.brand)
            }
        }
        HorizontalDivider(thickness = 0.5.dp, color = c.border)

        // iOS-style Segmented Pill Control
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 12.dp)
                .clip(RoundedCornerShape(999.dp))
                .background(c.bgSubtle)
                .border(0.5.dp, c.border, RoundedCornerShape(999.dp))
                .padding(3.dp),
        ) {
            listOf(
                "الطابعات (${printers.size})",
                "السجل (${jobs.size})",
                "التشخيص",
            ).forEachIndexed { idx, label ->
                val active = selectedTab == idx
                val bg by animateColorAsState(if (active) c.bgSurface else Color.Transparent, label = "seg_bg")
                Box(
                    modifier = Modifier
                        .weight(1f)
                        .clip(RoundedCornerShape(999.dp))
                        .background(bg)
                        .clickable {
                            selectedTab = idx
                            if (idx == 1) reloadData()
                        }
                        .padding(vertical = 8.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        text = label,
                        style = MaterialTheme.typography.labelLarge.copy(
                            fontWeight = if (active) FontWeight.Bold else FontWeight.Medium,
                        ),
                        color = if (active) c.textPrimary else c.textMuted,
                    )
                }
            }
        }

        when (selectedTab) {
            0 -> PrintersTab(
                printers = printers,
                onAddManual = { showAddPrinter = true },
                onConfigure = { editingPrinter = it },
                onTestPrint = { p ->
                    app.enqueueTestPrint(p.name)
                    reloadData()
                },
                onRequestUsbPermission = { p ->
                    UsbOtgTransport.requestPermission(ctx, p.address)
                },
                onSetDefault = { p ->
                    app.configStore.setDefaultPrinter(p.name)
                    app.discovery.invalidate()
                    reloadData(forcePrinters = true)
                },
            )
            1 -> QueueTab(
                jobs = jobs,
                onRetry = { id ->
                    app.queueEngine.retryJob(id)?.let { app.worker.wakePrinter(it.printerName) }
                    reloadData()
                },
                onCancel = { id ->
                    app.queueEngine.cancelJob(id)
                    app.worker.refreshSummary()
                    reloadData()
                },
                onClearFinished = {
                    app.queueEngine.clearFinished()
                    app.worker.refreshSummary()
                    reloadData()
                },
            )
            2 -> DiagnosticsTab(
                app = app,
                printers = printers,
                onExport = {
                    DiagnosticsExporter.exportAndShare(ctx, app.configStore, app.queueEngine, printers)
                },
            )
        }
    }

    if (showAddPrinter) {
        AddManualPrinterDialog(
            onDismiss = { showAddPrinter = false },
            onSave = { entry ->
                app.configStore.upsertManualPrinter(entry)
                app.discovery.invalidate()
                showAddPrinter = false
                reloadData(forcePrinters = true)
            },
        )
    }

    editingPrinter?.let { target ->
        val isManual = app.configStore.current().manualPrinters.any {
            it.name.equals(target.name, ignoreCase = true)
        }
        PrinterSettingsDialog(
            printer = target,
            initial = app.configStore.getPrinterSettings(target.name),
            onDismiss = { editingPrinter = null },
            onSave = { updated ->
                app.configStore.setPrinterSettings(target.name, updated)
                app.discovery.invalidate()
                editingPrinter = null
                reloadData(forcePrinters = true)
            },
            onDelete = if (isManual) {
                {
                    app.configStore.removeManualPrinter(target.name)
                    if (app.configStore.current().defaultPrinter.equals(target.name, ignoreCase = true)) {
                        app.configStore.setDefaultPrinter(null)
                    }
                    app.discovery.invalidate()
                    editingPrinter = null
                    reloadData(forcePrinters = true)
                }
            } else {
                null
            },
        )
    }
}

@Composable
private fun PrintersTab(
    printers: List<PrinterInfo>,
    onAddManual: () -> Unit,
    onConfigure: (PrinterInfo) -> Unit,
    onTestPrint: (PrinterInfo) -> Unit,
    onRequestUsbPermission: (PrinterInfo) -> Unit,
    onSetDefault: (PrinterInfo) -> Unit,
) {
    val c = LocalYaseirColors.current
    LazyColumn(
        modifier = Modifier.fillMaxSize().padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        item {
            Button(
                onClick = onAddManual,
                colors = ButtonDefaults.buttonColors(containerColor = c.brand, contentColor = Color.White),
                shape = RoundedCornerShape(999.dp),
                modifier = Modifier.fillMaxWidth().height(44.dp),
            ) {
                Icon(Icons.Rounded.Add, contentDescription = null, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(6.dp))
                Text("إضافة طابعة")
            }
        }
        items(printers, key = { it.name }) { p ->
            val needsUsbPerm = p.transport == PrinterTransportType.USB_OTG && p.status == "permission_required"
            val dotColor = when {
                needsUsbPerm -> c.warnFg
                p.online -> c.okFg
                else -> c.errFg
            }
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(18.dp))
                    .background(c.bgSurface)
                    .border(if (p.isDefault) 1.dp else 0.75.dp, if (p.isDefault) c.brand else c.border, RoundedCornerShape(18.dp))
                    .padding(16.dp),
            ) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    Box(
                        modifier = Modifier
                            .size(9.dp)
                            .clip(CircleShape)
                            .background(dotColor),
                    )
                    Column(modifier = Modifier.weight(1f)) {
                        Text(p.displayName, style = MaterialTheme.typography.titleSmall, color = c.textPrimary)
                        Text(
                            "${p.transport.wireName.uppercase()} • ${p.protocol.wireName.uppercase()} • ${p.address}",
                            style = MaterialTheme.typography.bodySmall,
                            color = c.textMuted,
                        )
                    }
                    if (p.isDefault) {
                        Text(
                            "افتراضية",
                            style = MaterialTheme.typography.labelSmall,
                            color = c.okFg,
                            modifier = Modifier
                                .clip(RoundedCornerShape(999.dp))
                                .background(c.okBg)
                                .padding(horizontal = 9.dp, vertical = 3.dp),
                        )
                    }
                }
                Spacer(Modifier.height(12.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (needsUsbPerm) {
                        Button(
                            onClick = { onRequestUsbPermission(p) },
                            colors = ButtonDefaults.buttonColors(containerColor = c.brand, contentColor = Color.White),
                            shape = RoundedCornerShape(999.dp),
                        ) {
                            Icon(Icons.Rounded.Usb, contentDescription = null, modifier = Modifier.size(16.dp))
                            Spacer(Modifier.width(4.dp))
                            Text("تفعيل USB")
                        }
                    } else {
                        OutlinedButton(onClick = { onTestPrint(p) }, shape = RoundedCornerShape(999.dp)) {
                            Icon(Icons.Rounded.Print, contentDescription = null, modifier = Modifier.size(15.dp))
                            Spacer(Modifier.width(4.dp))
                            Text("اختبار")
                        }
                    }
                    OutlinedButton(onClick = { onConfigure(p) }, shape = RoundedCornerShape(999.dp)) {
                        Icon(Icons.Rounded.Tune, contentDescription = null, modifier = Modifier.size(15.dp))
                        Spacer(Modifier.width(4.dp))
                        Text("ضبط")
                    }
                    if (!p.isDefault) {
                        OutlinedButton(onClick = { onSetDefault(p) }, shape = RoundedCornerShape(999.dp)) {
                            Text("افتراضية")
                        }
                    }
                }
            }
        }
        item { Spacer(Modifier.height(16.dp)) }
    }
}

@Composable
private fun QueueTab(
    jobs: List<PrintJobRecord>,
    onRetry: (String) -> Unit,
    onCancel: (String) -> Unit,
    onClearFinished: () -> Unit,
) {
    val c = LocalYaseirColors.current
    LazyColumn(
        modifier = Modifier.fillMaxSize().padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        if (jobs.any { it.state.isTerminal }) {
            item {
                OutlinedButton(
                    onClick = onClearFinished,
                    shape = RoundedCornerShape(999.dp),
                    modifier = Modifier.fillMaxWidth().height(42.dp),
                ) {
                    Icon(Icons.Rounded.DeleteSweep, contentDescription = null, modifier = Modifier.size(18.dp))
                    Spacer(Modifier.width(6.dp))
                    Text("مسح المكتملة")
                }
            }
        }
        if (jobs.isEmpty()) {
            item {
                Box(
                    modifier = Modifier.fillMaxWidth().padding(vertical = 48.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    Text("لا توجد مهام طباعة", style = MaterialTheme.typography.bodyMedium, color = c.textMuted)
                }
            }
        }
        items(jobs, key = { it.id }) { job ->
            val (badgeColor, badgeBg, badgeLabel) = when (job.state) {
                JobState.COMPLETED -> Triple(c.okFg, c.okBg, "مكتملة")
                JobState.FAILED -> Triple(c.errFg, c.errBg, "فشلت")
                JobState.CANCELLED -> Triple(c.textMuted, c.bgSubtle, "ملغاة")
                else -> Triple(c.warnFg, c.warnBg, "جاري الطباعة")
            }
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(18.dp))
                    .background(c.bgSurface)
                    .border(0.75.dp, c.border, RoundedCornerShape(18.dp))
                    .padding(16.dp),
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Column(modifier = Modifier.weight(1f)) {
                        Text(job.printerName, style = MaterialTheme.typography.titleSmall, color = c.textPrimary)
                        Text(
                            "${job.sourceKind.uppercase()} • ×${job.copies} • ${job.attempts}/${job.maxAttempts}",
                            style = MaterialTheme.typography.bodySmall,
                            color = c.textMuted,
                        )
                    }
                    Text(
                        badgeLabel,
                        style = MaterialTheme.typography.labelSmall.copy(fontWeight = FontWeight.Bold),
                        color = badgeColor,
                        modifier = Modifier
                            .clip(RoundedCornerShape(999.dp))
                            .background(badgeBg)
                            .padding(horizontal = 10.dp, vertical = 4.dp),
                    )
                }
                if (!job.errorMessage.isNullOrBlank()) {
                    Spacer(Modifier.height(6.dp))
                    Text(
                        "${job.errorCode ?: "ERR"}: ${job.errorMessage}",
                        style = MaterialTheme.typography.bodySmall,
                        color = c.errFg,
                    )
                }
                if (job.state == JobState.FAILED || job.state.isActive) {
                    Spacer(Modifier.height(10.dp))
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        if (job.state == JobState.FAILED) {
                            OutlinedButton(onClick = { onRetry(job.id) }, shape = RoundedCornerShape(999.dp)) {
                                Text("إعادة")
                            }
                        }
                        if (job.state.isActive) {
                            OutlinedButton(onClick = { onCancel(job.id) }, shape = RoundedCornerShape(999.dp)) {
                                Text("إلغاء", color = c.errFg)
                            }
                        }
                    }
                }
            }
        }
        item { Spacer(Modifier.height(16.dp)) }
    }
}

@Composable
private fun DiagnosticsTab(
    app: YaseirPrintApp,
    printers: List<PrinterInfo>,
    onExport: () -> Unit,
) {
    val c = LocalYaseirColors.current
    val snap = app.configStore.current()
    Column(
        modifier = Modifier.fillMaxSize().padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(18.dp))
                .background(c.bgSurface)
                .border(0.75.dp, c.border, RoundedCornerShape(18.dp))
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            DiagRow("الخادم", snap.origin ?: "—")
            HorizontalDivider(thickness = 0.5.dp, color = c.border)
            DiagRow("الإصدار", "Android v${BuildConfig.VERSION_NAME}")
            HorizontalDivider(thickness = 0.5.dp, color = c.border)
            DiagRow("الطابعات", "${printers.size}")
            HorizontalDivider(thickness = 0.5.dp, color = c.border)
            DiagRow("تحديث الكاش", "#${app.discovery.capsEpoch()}")
        }

        Button(
            onClick = onExport,
            colors = ButtonDefaults.buttonColors(containerColor = c.brand, contentColor = Color.White),
            shape = RoundedCornerShape(999.dp),
            modifier = Modifier.fillMaxWidth().height(46.dp),
        ) {
            Icon(Icons.Rounded.Share, contentDescription = null, modifier = Modifier.size(18.dp))
            Spacer(Modifier.width(6.dp))
            Text("تصدير التشخيص")
        }
    }
}

@Composable
private fun DiagRow(label: String, value: String) {
    val c = LocalYaseirColors.current
    Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
        Text(label, style = MaterialTheme.typography.bodySmall, color = c.textMuted)
        Text(value, style = MaterialTheme.typography.bodySmall.copy(fontWeight = FontWeight.Medium), color = c.textPrimary)
    }
}
