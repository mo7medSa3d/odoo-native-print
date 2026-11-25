package com.company.odoonativeprint.ui.settings
import com.company.odoonativeprint.ui.theme.tr

import androidx.compose.animation.animateColorAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.company.odoonativeprint.core.AppConfigStore
import com.company.odoonativeprint.ui.browser.explainOriginError
import com.company.odoonativeprint.ui.theme.LocalYaseirColors

/**
 * Minimal iOS-style grouped Settings screen matching desktop app/dist/settings.html.
 */
@Composable
fun SettingsScreen(
    configStore: AppConfigStore,
    onOriginUpdated: (String) -> Unit,
    onClearWebData: () -> Unit,
    onBack: () -> Unit,
) {
    val c = LocalYaseirColors.current
    val snap by configStore.state.collectAsState()

    var originInput by remember(snap.origin) { mutableStateOf(snap.origin ?: "") }
    var autoReprint by remember(snap.autoReprint) { mutableStateOf(snap.autoReprint) }
    var feedback by remember { mutableStateOf<String?>(null) }
    var isError by remember { mutableStateOf(false) }
    var confirmClear by remember { mutableStateOf(false) }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(c.bgCanvas)
            .statusBarsPadding()
            .navigationBarsPadding()
            .imePadding(),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .background(c.bgSurface)
                .padding(horizontal = 12.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = onBack) {
                Icon(Icons.AutoMirrored.Rounded.ArrowBack, contentDescription = tr("رجوع"), tint = c.textPrimary)
            }
            Text(tr("الإعدادات"), style = MaterialTheme.typography.titleMedium, color = c.textPrimary)
        }
        HorizontalDivider(thickness = 0.5.dp, color = c.border)

        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            // Server Card
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(18.dp))
                    .background(c.bgSurface)
                    .border(0.75.dp, c.border, RoundedCornerShape(18.dp))
                    .padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text(tr("خادم Odoo"), style = MaterialTheme.typography.titleSmall, color = c.textPrimary)
                OutlinedTextField(
                    value = originInput,
                    onValueChange = { originInput = it; feedback = null },
                    placeholder = { Text("https://erp.company.com") },
                    singleLine = true,
                    shape = RoundedCornerShape(14.dp),
                    modifier = Modifier.fillMaxWidth(),
                )
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                    Text(tr("إعادة الطباعة تلقائيًا"), style = MaterialTheme.typography.bodyMedium, color = c.textPrimary, modifier = Modifier.weight(1f))
                    Switch(checked = autoReprint, onCheckedChange = { autoReprint = it })
                }
                if (feedback != null) {
                    Text(
                        feedback!!,
                        style = MaterialTheme.typography.bodySmall,
                        color = if (isError) c.errFg else c.okFg,
                    )
                }
                val savedLabel = tr("تم الحفظ")
                Button(
                    onClick = {
                        configStore.saveOrigin(originInput, autoReprint)
                            .onSuccess { normalized ->
                                isError = false
                                feedback = savedLabel
                                onOriginUpdated(normalized)
                            }
                            .onFailure {
                                isError = true
                                feedback = explainOriginError(it.message, snap.language)
                            }
                    },
                    colors = ButtonDefaults.buttonColors(containerColor = c.brand, contentColor = Color.White),
                    shape = RoundedCornerShape(999.dp),
                    modifier = Modifier.fillMaxWidth().height(44.dp),
                ) {
                    Text(tr("حفظ"))
                }
            }

            // Appearance & Language Segmented Pills
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(18.dp))
                    .background(c.bgSurface)
                    .border(0.75.dp, c.border, RoundedCornerShape(18.dp))
                    .padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text(tr("المظهر"), style = MaterialTheme.typography.titleSmall, color = c.textPrimary)
                SegmentedPillRow(
                    options = listOf("system" to tr("تلقائي"), "light" to tr("فاتح"), "dark" to tr("داكن")),
                    selected = snap.themeMode,
                    onSelect = { configStore.setThemeMode(it) },
                )
                HorizontalDivider(thickness = 0.5.dp, color = c.border)
                Text(tr("اللغة"), style = MaterialTheme.typography.titleSmall, color = c.textPrimary)
                SegmentedPillRow(
                    options = listOf("ar" to tr("العربية"), "en" to "English"),
                    selected = snap.language,
                    onSelect = { configStore.setLanguage(it) },
                )
            }

            // Web Data Wipe
            OutlinedButton(
                onClick = { confirmClear = true },
                shape = RoundedCornerShape(999.dp),
                modifier = Modifier.fillMaxWidth().height(46.dp),
            ) {
                Text(tr("مسح بيانات التصفح"), color = c.errFg)
            }
        }
    }

    if (confirmClear) {
        AlertDialog(
            onDismissRequest = { confirmClear = false },
            shape = RoundedCornerShape(22.dp),
            title = { Text(tr("مسح بيانات التصفح؟")) },
            text = { Text(tr("سيتم تسجيل خروجك من Odoo.")) },
            confirmButton = {
                TextButton(
                    onClick = {
                        confirmClear = false
                        onClearWebData()
                    },
                ) { Text(tr("مسح"), color = c.errFg) }
            },
            dismissButton = { TextButton(onClick = { confirmClear = false }) { Text(tr("إلغاء")) } },
        )
    }
}

@Composable
private fun SegmentedPillRow(
    options: List<Pair<String, String>>,
    selected: String,
    onSelect: (String) -> Unit,
) {
    val c = LocalYaseirColors.current
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(999.dp))
            .background(c.bgSubtle)
            .border(0.5.dp, c.border, RoundedCornerShape(999.dp))
            .padding(3.dp),
    ) {
        options.forEach { (key, label) ->
            val active = selected == key
            val bg by animateColorAsState(if (active) c.bgSurface else Color.Transparent, label = "pill_bg")
            Box(
                modifier = Modifier
                    .weight(1f)
                    .clip(RoundedCornerShape(999.dp))
                    .background(bg)
                    .clickable { onSelect(key) }
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
}
