package com.company.odoonativeprint.ui.controlcenter

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.company.odoonativeprint.core.ManualPrinterEntry
import com.company.odoonativeprint.core.PrinterInfo
import com.company.odoonativeprint.core.PrinterProtocol
import com.company.odoonativeprint.core.PrinterSettings
import com.company.odoonativeprint.core.PrinterTransportType
import com.company.odoonativeprint.ui.theme.LocalYaseirColors

@Composable
fun AddManualPrinterDialog(
    onDismiss: () -> Unit,
    onSave: (ManualPrinterEntry) -> Unit,
) {
    var name by remember { mutableStateOf("") }
    var address by remember { mutableStateOf("192.168.1.200:9100") }
    var transport by remember { mutableStateOf(PrinterTransportType.TCP_9100) }
    var protocol by remember { mutableStateOf(PrinterProtocol.ESCPOS) }
    var paperWidth by remember { mutableStateOf("80") }

    AlertDialog(
        onDismissRequest = onDismiss,
        shape = RoundedCornerShape(22.dp),
        title = { Text("إضافة طابعة") },
        text = {
            Column(
                modifier = Modifier.verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it },
                    label = { Text("الاسم") },
                    singleLine = true,
                    shape = RoundedCornerShape(12.dp),
                    modifier = Modifier.fillMaxWidth(),
                )
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    listOf(
                        PrinterTransportType.TCP_9100 to "TCP 9100",
                        PrinterTransportType.IPP to "IPP",
                        PrinterTransportType.BLUETOOTH_SPP to "BT",
                    ).forEach { (t, label) ->
                        FilterChip(
                            selected = transport == t,
                            onClick = {
                                transport = t
                                address = when (t) {
                                    PrinterTransportType.TCP_9100 -> "192.168.1.200:9100"
                                    PrinterTransportType.IPP -> "ipp://192.168.1.200:631/ipp/print"
                                    else -> "bt://00:11:22:33:44:55"
                                }
                            },
                            label = { Text(label) },
                        )
                    }
                }
                OutlinedTextField(
                    value = address,
                    onValueChange = { address = it },
                    label = { Text("العنوان") },
                    singleLine = true,
                    shape = RoundedCornerShape(12.dp),
                    modifier = Modifier.fillMaxWidth(),
                )
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    listOf(PrinterProtocol.ESCPOS, PrinterProtocol.ZPL, PrinterProtocol.TSPL, PrinterProtocol.PDF).forEach { p ->
                        FilterChip(
                            selected = protocol == p,
                            onClick = { protocol = p },
                            label = { Text(p.wireName.uppercase()) },
                        )
                    }
                }
                OutlinedTextField(
                    value = paperWidth,
                    onValueChange = { paperWidth = it.filter(Char::isDigit) },
                    label = { Text("العرض (mm)") },
                    singleLine = true,
                    shape = RoundedCornerShape(12.dp),
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        },
        confirmButton = {
            TextButton(
                onClick = {
                    if (name.isNotBlank() && address.isNotBlank()) {
                        onSave(
                            ManualPrinterEntry(
                                name = name.trim(),
                                transport = transport,
                                address = address.trim(),
                                protocol = protocol,
                                paperWidthMm = paperWidth.toIntOrNull()?.coerceIn(32, 297) ?: 80,
                            ),
                        )
                    }
                },
            ) { Text("حفظ") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("إلغاء") } },
    )
}

@Composable
fun PrinterSettingsDialog(
    printer: PrinterInfo,
    initial: PrinterSettings,
    onDismiss: () -> Unit,
    onSave: (PrinterSettings) -> Unit,
    onDelete: (() -> Unit)? = null,
) {
    val c = LocalYaseirColors.current
    var protocol by remember { mutableStateOf(initial.protocol) }
    var paperWidth by remember { mutableStateOf(initial.paperWidthMm.toString()) }
    var dpi by remember { mutableStateOf(initial.dpi.toString()) }
    var cutPaper by remember { mutableStateOf(initial.cutPaper) }
    var openDrawer by remember { mutableStateOf(initial.openCashDrawer) }
    var dithering by remember { mutableStateOf(initial.dithering) }

    AlertDialog(
        onDismissRequest = onDismiss,
        shape = RoundedCornerShape(22.dp),
        title = { Text(printer.displayName) },
        text = {
            Column(
                modifier = Modifier.verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    listOf(PrinterProtocol.AUTO, PrinterProtocol.ESCPOS, PrinterProtocol.ZPL, PrinterProtocol.TSPL).forEach { p ->
                        FilterChip(
                            selected = protocol == p,
                            onClick = { protocol = p },
                            label = { Text(p.wireName.uppercase()) },
                        )
                    }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedTextField(
                        value = paperWidth,
                        onValueChange = { paperWidth = it.filter(Char::isDigit) },
                        label = { Text("العرض mm") },
                        singleLine = true,
                        shape = RoundedCornerShape(12.dp),
                        modifier = Modifier.weight(1f),
                    )
                    OutlinedTextField(
                        value = dpi,
                        onValueChange = { dpi = it.filter(Char::isDigit) },
                        label = { Text("DPI") },
                        singleLine = true,
                        shape = RoundedCornerShape(12.dp),
                        modifier = Modifier.weight(1f),
                    )
                }
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                    Text("قص تلقائي", modifier = Modifier.weight(1f))
                    Switch(checked = cutPaper, onCheckedChange = { cutPaper = it })
                }
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                    Text("درج النقدية", modifier = Modifier.weight(1f))
                    Switch(checked = openDrawer, onCheckedChange = { openDrawer = it })
                }
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                    Text("تحسين الصور", modifier = Modifier.weight(1f))
                    Switch(checked = dithering, onCheckedChange = { dithering = it })
                }
                if (onDelete != null) {
                    OutlinedButton(
                        onClick = onDelete,
                        shape = RoundedCornerShape(999.dp),
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Icon(
                            Icons.Rounded.Delete,
                            contentDescription = null,
                            tint = c.errFg,
                            modifier = Modifier.size(18.dp),
                        )
                        Spacer(Modifier.width(6.dp))
                        Text("حذف الطابعة", color = c.errFg)
                    }
                }
            }
        },
        confirmButton = {
            TextButton(
                onClick = {
                    onSave(
                        initial.copy(
                            protocol = protocol,
                            paperWidthMm = paperWidth.toIntOrNull()?.coerceIn(32, 297) ?: 80,
                            dpi = dpi.toIntOrNull()?.coerceIn(150, 600) ?: 203,
                            cutPaper = cutPaper,
                            openCashDrawer = openDrawer,
                            dithering = dithering,
                        ),
                    )
                },
            ) { Text("حفظ") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("إلغاء") } },
    )
}
