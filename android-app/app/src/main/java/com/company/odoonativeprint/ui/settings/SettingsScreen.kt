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
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.text.style.TextOverflow
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
import android.content.Intent
import android.net.Uri
import android.os.Build
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import com.company.odoonativeprint.BuildConfig
import com.company.odoonativeprint.core.AppConfigStore
import com.company.odoonativeprint.core.LicenseManager
import com.company.odoonativeprint.core.LicenseResult
import com.company.odoonativeprint.ui.browser.explainOriginError
import com.company.odoonativeprint.ui.theme.LocalItbaaColors

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
    val c = LocalItbaaColors.current
    val context = LocalContext.current
    val clipboard = LocalClipboardManager.current
    val snap by configStore.state.collectAsState()
    var sysInfoCopied by remember { mutableStateOf(false) }

    var originInput by remember(snap.origin) { mutableStateOf(snap.origin ?: "") }
    var autoReprint by remember(snap.autoReprint) { mutableStateOf(snap.autoReprint) }
    var feedback by remember { mutableStateOf<String?>(null) }
    var isError by remember { mutableStateOf(false) }
    var confirmClear by remember { mutableStateOf(false) }
    var licenseInput by remember(snap.licenseKey) { mutableStateOf(snap.licenseKey ?: "") }
    var licenseFeedback by remember { mutableStateOf<String?>(null) }

    val nowUtc = remember(snap.licenseKey, snap.origin, snap.lastSeenUtc) {
        LicenseManager.nowUtcSecs()
    }
    val licenseStatus = remember(snap.licenseKey, snap.origin, snap.lastSeenUtc, nowUtc) {
        LicenseManager.verify(
            key = snap.licenseKey,
            configuredOrigin = snap.origin,
            nowUtc = nowUtc,
            lastSeenUtc = snap.lastSeenUtc,
        )
    }

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

        Box(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState()),
            contentAlignment = Alignment.TopCenter,
        ) {
        Column(
            modifier = Modifier
                .widthIn(max = 720.dp)
                .fillMaxWidth()
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

            // License & Activation Card
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(18.dp))
                    .background(c.bgSurface)
                    .border(0.75.dp, c.border, RoundedCornerShape(18.dp))
                    .padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text(tr("الترخيص والتفعيل"), style = MaterialTheme.typography.titleSmall, color = c.textPrimary)

                when (val st = licenseStatus) {
                    is LicenseResult.Valid -> {
                        val payload = st.payload
                        val days = payload.daysRemaining(nowUtc)
                        val tierLabel = if (payload.tier == "pro") "Pro" else tr("تجريبي")
                        val remainingText = when {
                            days == null -> tr("دائم")
                            days == 1L -> tr("يوم واحد")
                            days in 2L..10L -> "$days ${tr("أيام")}"
                            else -> "$days ${tr("يوم")}"
                        }
                        val statusColor = if (days != null && days <= 7L) c.warnFg else c.okFg
                        LicenseInfoRow(label = tr("الحالة"), value = tierLabel, valueColor = statusColor)
                        LicenseInfoRow(label = tr("العميل"), value = payload.customer, valueColor = c.textPrimary)
                        LicenseInfoRow(label = tr("المتبقي"), value = remainingText, valueColor = statusColor)
                    }
                    is LicenseResult.Invalid -> {
                        val expired = st.code == LicenseManager.ERR_LICENSE_EXPIRED
                        LicenseInfoRow(
                            label = tr("الحالة"),
                            value = if (expired) tr("منتهي الصلاحية") else tr("غير مفعّل"),
                            valueColor = if (expired) c.errFg else c.warnFg,
                        )
                    }
                }

                HorizontalDivider(thickness = 0.5.dp, color = c.border)

                OutlinedTextField(
                    value = licenseInput,
                    onValueChange = { licenseInput = it; licenseFeedback = null },
                    singleLine = true,
                    label = { Text(tr("مفتاح التفعيل (ITBAA1...)")) },
                    placeholder = { Text("ITBAA1....") },
                    modifier = Modifier.fillMaxWidth(),
                    shape = RoundedCornerShape(14.dp),
                )
                val licenseOkLabel = tr("تم تفعيل الرخصة بنجاح")
                val licenseExpiredLabel = tr("مفتاح الترخيص منتهي الصلاحية")
                val licenseInvalidLabel = tr("مفتاح غير صالح أو لا يطابق عنوان أودو")
                Button(
                    onClick = {
                        val now = LicenseManager.nowUtcSecs()
                        val res = LicenseManager.verify(
                            licenseInput,
                            snap.origin,
                            now,
                            snap.lastSeenUtc,
                        )
                        if (res.isValid) {
                            configStore.saveLicenseKey(licenseInput.trim())
                            configStore.updateLastSeenUtc(now)
                            licenseFeedback = licenseOkLabel
                        } else {
                            val inv = res as LicenseResult.Invalid
                            licenseFeedback = if (inv.code == LicenseManager.ERR_LICENSE_EXPIRED) {
                                licenseExpiredLabel
                            } else {
                                licenseInvalidLabel
                            }
                        }
                    },
                    colors = ButtonDefaults.buttonColors(containerColor = c.brand, contentColor = Color.White),
                    shape = RoundedCornerShape(999.dp),
                    modifier = Modifier.fillMaxWidth().height(46.dp),
                ) {
                    Text(tr("حفظ وتفعيل الرخصة"))
                }
                licenseFeedback?.let { fb ->
                    Text(fb, style = MaterialTheme.typography.bodySmall, color = c.textSecondary)
                }
            }

            // Web Data Wipe

            // About & Support card (ownership, technical summary, WhatsApp & Email)
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(18.dp))
                    .background(c.bgSurface)
                    .border(0.5.dp, c.border, RoundedCornerShape(18.dp))
                    .padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Text(
                    tr("حول التطبيق والدعم"),
                    style = MaterialTheme.typography.titleMedium.copy(fontWeight = FontWeight.Bold),
                    color = c.textPrimary,
                )
                Text(
                    tr("محرك طباعة صامتة ومباشرة لبيئات Odoo (17–20) مبني أصلياً بلغة Kotlin بدون IoT Box، بطابور SQLite WAL محلي وترخيص Ed25519 أوفلاين."),
                    style = MaterialTheme.typography.bodySmall,
                    color = c.textSecondary,
                )
                LicenseInfoRow(tr("الإصدار"), "v${BuildConfig.VERSION_NAME} · Android ${Build.VERSION.RELEASE}", c.textPrimary)
                LicenseInfoRow(tr("واتساب الدعم"), "+20 100 778 1842", c.textPrimary)
                LicenseInfoRow(tr("البريد الإلكتروني"), "itbaa.odoo@gmail.com", c.textPrimary)
                Row(
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Button(
                        onClick = {
                            val summary = "Itbaa Android | v${BuildConfig.VERSION_NAME} | Android ${Build.VERSION.RELEASE}"
                            val url = "https://wa.me/201007781842?text=" + Uri.encode(summary)
                            runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }
                        },
                        colors = ButtonDefaults.buttonColors(containerColor = c.brand, contentColor = Color.White),
                        shape = RoundedCornerShape(999.dp),
                        modifier = Modifier.weight(1f).height(44.dp),
                    ) {
                        Text(tr("تواصل عبر واتساب"), maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                    OutlinedButton(
                        onClick = {
                            val uri = Uri.parse("mailto:itbaa.odoo@gmail.com?subject=" + Uri.encode("Itbaa Android Support"))
                            runCatching { context.startActivity(Intent(Intent.ACTION_SENDTO, uri)) }
                        },
                        shape = RoundedCornerShape(999.dp),
                        modifier = Modifier.weight(1f).height(44.dp),
                    ) {
                        Text(tr("إرسال بريد"), maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                }
                OutlinedButton(
                    onClick = {
                        val info = "Itbaa Android | v${BuildConfig.VERSION_NAME} | Android ${Build.VERSION.RELEASE} | AppId: ${BuildConfig.APPLICATION_ID}"
                        clipboard.setText(AnnotatedString(info))
                        sysInfoCopied = true
                    },
                    shape = RoundedCornerShape(999.dp),
                    modifier = Modifier.fillMaxWidth().height(42.dp),
                ) {
                    Text(if (sysInfoCopied) tr("تم نسخ معلومات النظام") else tr("نسخ معلومات النظام"))
                }
                Text(
                    tr("تطوير وإشراف اطبع · جميع الحقوق محفوظة © 2026"),
                    style = MaterialTheme.typography.labelSmall,
                    color = c.textMuted,
                )
            }

            OutlinedButton(
                onClick = { confirmClear = true },
                shape = RoundedCornerShape(999.dp),
                modifier = Modifier.fillMaxWidth().height(46.dp),
            ) {
                Text(tr("مسح بيانات التصفح"), color = c.errFg)
            }
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
private fun LicenseInfoRow(
    label: String,
    value: String,
    valueColor: Color,
) {
    val c = LocalItbaaColors.current
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.bodyMedium,
            color = c.textSecondary,
        )
        Text(
            text = value,
            style = MaterialTheme.typography.bodyMedium.copy(fontWeight = FontWeight.SemiBold),
            color = valueColor,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun SegmentedPillRow(
    options: List<Pair<String, String>>,
    selected: String,
    onSelect: (String) -> Unit,
) {
    val c = LocalItbaaColors.current
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
                    .padding(horizontal = 6.dp, vertical = 8.dp),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    text = label,
                    style = MaterialTheme.typography.labelLarge.copy(
                        fontWeight = if (active) FontWeight.Bold else FontWeight.Medium,
                    ),
                    color = if (active) c.textPrimary else c.textMuted,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}
