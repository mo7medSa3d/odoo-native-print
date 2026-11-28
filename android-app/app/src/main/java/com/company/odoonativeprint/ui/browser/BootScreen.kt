package com.company.odoonativeprint.ui.browser
import com.company.odoonativeprint.ui.theme.LocalLanguage
import com.company.odoonativeprint.ui.theme.localize
import com.company.odoonativeprint.ui.theme.tr

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
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
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.company.odoonativeprint.BuildConfig
import com.company.odoonativeprint.R
import com.company.odoonativeprint.core.OriginPolicy
import com.company.odoonativeprint.ui.theme.LocalItbaaColors
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.net.HttpURLConnection
import java.net.URL

/**
 * Minimal SaaS first-run connection screen matching Itbaa desktop identity.
 */
@Composable
fun BootScreen(
    initialOrigin: String = "",
    initialLicenseKey: String = "",
    onLicenseSaved: (String) -> Unit = {},
    onOriginSaved: (String) -> Unit,
) {
    val c = LocalItbaaColors.current
    val currentLanguage = LocalLanguage.current
    val scope = rememberCoroutineScope()
    var originInput by remember(initialOrigin) { mutableStateOf(initialOrigin) }
    var licenseInput by remember(initialLicenseKey) { mutableStateOf(initialLicenseKey) }
    var statusText by remember { mutableStateOf<String?>(null) }
    var isError by remember { mutableStateOf(false) }
    var probing by remember { mutableStateOf(false) }

    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(c.bgCanvas)
            .statusBarsPadding()
            .navigationBarsPadding()
            .imePadding()
            .verticalScroll(rememberScrollState())
            .padding(24.dp),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            modifier = Modifier
                .widthIn(max = 420.dp)
                .fillMaxWidth()
                .clip(RoundedCornerShape(24.dp))
                .background(c.bgSurface)
                .border(0.75.dp, c.border, RoundedCornerShape(24.dp))
                .padding(28.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Image(
                painter = painterResource(R.drawable.ic_itbaa_logo),
                contentDescription = "Itbaa",
                modifier = Modifier
                    .size(60.dp)
                    .clip(RoundedCornerShape(16.dp)),
            )
            Spacer(Modifier.height(14.dp))
            Text(
                text = "Itbaa",
                style = MaterialTheme.typography.headlineLarge,
                color = c.textPrimary,
            )
            Text(
                text = "v${BuildConfig.VERSION_NAME}",
                style = MaterialTheme.typography.labelSmall,
                color = c.textMuted,
            )
            Spacer(Modifier.height(24.dp))

            OutlinedTextField(
                value = originInput,
                onValueChange = {
                    originInput = it
                    statusText = null
                },
                label = { Text(tr("خادم Odoo")) },
                placeholder = { Text("https://erp.company.com") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
                shape = RoundedCornerShape(14.dp),
                colors = OutlinedTextFieldDefaults.colors(
                    focusedBorderColor = c.brand,
                    unfocusedBorderColor = c.border,
                    focusedContainerColor = c.bgElevated,
                    unfocusedContainerColor = c.bgElevated,
                ),
            )

            Spacer(Modifier.height(12.dp))
            OutlinedTextField(
                value = licenseInput,
                onValueChange = { licenseInput = it },
                label = { Text(tr("مفتاح التفعيل (ITBAA1...)")) },
                placeholder = { Text("ITBAA1....") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
                shape = RoundedCornerShape(14.dp),
                colors = OutlinedTextFieldDefaults.colors(
                    focusedBorderColor = c.brand,
                    unfocusedBorderColor = c.border,
                    focusedContainerColor = c.bgElevated,
                    unfocusedContainerColor = c.bgElevated,
                ),
            )

            AnimatedVisibility(visible = statusText != null) {
                Text(
                    text = statusText.orEmpty(),
                    style = MaterialTheme.typography.bodySmall.copy(fontWeight = FontWeight.Medium),
                    color = if (isError) c.errFg else c.okFg,
                    modifier = Modifier
                        .padding(top = 12.dp)
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(12.dp))
                        .background(if (isError) c.errBg else c.okBg)
                        .padding(horizontal = 12.dp, vertical = 8.dp),
                )
            }

            Spacer(Modifier.height(20.dp))
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                val connectedLabel = tr("متصل")
                val connectionFailedLabel = tr("تعذّر الاتصال")
                OutlinedButton(
                    onClick = {
                        OriginPolicy.validateAndNormalize(originInput)
                            .onFailure {
                                isError = true
                                statusText = explainOriginError(it.message, currentLanguage)
                            }
                            .onSuccess { normalized ->
                                probing = true
                                statusText = null
                                scope.launch {
                                    val ok = probeOrigin(normalized)
                                    probing = false
                                    isError = !ok
                                    statusText = if (ok) connectedLabel else connectionFailedLabel
                                }
                            }
                    },
                    enabled = !probing,
                    shape = RoundedCornerShape(999.dp),
                    contentPadding = PaddingValues(horizontal = 12.dp),
                    modifier = Modifier.weight(1f).height(46.dp),
                ) {
                    if (probing) {
                        CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
                    } else {
                        Text(tr("اختبار"), maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                }

                Button(
                    onClick = {
                        OriginPolicy.validateAndNormalize(originInput)
                            .onSuccess { normalized ->
                                if (licenseInput.isNotBlank()) onLicenseSaved(licenseInput.trim())
                                onOriginSaved(normalized)
                            }
                            .onFailure {
                                isError = true
                                statusText = explainOriginError(it.message, currentLanguage)
                            }
                    },
                    colors = ButtonDefaults.buttonColors(containerColor = c.brand, contentColor = Color.White),
                    shape = RoundedCornerShape(999.dp),
                    contentPadding = PaddingValues(horizontal = 12.dp),
                    modifier = Modifier.weight(1.3f).height(46.dp),
                ) {
                    Text(tr("متابعة"), maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
        }
    }
}

internal fun explainOriginError(code: String?, language: String = "ar"): String = when (code) {
    "ORIGIN_HTTP_REQUIRES_IP" -> localize("استخدم HTTPS للنطاقات أو HTTP لعنوان IP فقط", language)
    "ORIGIN_METADATA_FORBIDDEN" -> localize("العنوان محظور أمنيًا", language)
    "ORIGIN_CREDENTIALS_FORBIDDEN" -> localize("أزل بيانات الدخول من الرابط", language)
    "ORIGIN_MISSING_SCHEME", "ORIGIN_UNSUPPORTED_SCHEME" -> localize("ابدأ بـ https:// أو http://", language)
    else -> localize("رابط غير صالح", language)
}

private suspend fun probeOrigin(origin: String): Boolean = withContext(Dispatchers.IO) {
    try {
        val conn = (URL(origin).openConnection() as HttpURLConnection).apply {
            requestMethod = "HEAD"
            connectTimeout = 4_000
            readTimeout = 4_000
            instanceFollowRedirects = true
        }
        try {
            conn.responseCode in 200..499
        } finally {
            conn.disconnect()
        }
    } catch (_: Exception) {
        false
    }
}
