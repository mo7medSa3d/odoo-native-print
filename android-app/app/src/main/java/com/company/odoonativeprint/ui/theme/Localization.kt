package com.company.odoonativeprint.ui.theme

import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf

/**
 * Locale for the Compose chrome. The app used to store an "en" preference but
 * every label was a hardcoded Arabic literal, so English never appeared.
 * Keep Arabic as the source language and translate the visible chrome here.
 */
val LocalLanguage = compositionLocalOf { "ar" }

private val ENGLISH = mapOf(
    "أزل بيانات الدخول من الرابط" to "Clear credentials from link",
    "إضافة طابعة" to "Add printer",
    "إظهار الشريط" to "Show toolbar",
    "إعادة" to "Retry",
    "إعادة الطباعة تلقائيًا" to "Auto reprint",
    "إغلاق التبويب" to "Close tab",
    "إلغاء" to "Cancel",
    "ابدأ بـ https:// أو http://" to "Start with https:// or http://",
    "اختبار" to "Test",
    "استخدم HTTPS للنطاقات أو HTTP لعنوان IP فقط" to "Use HTTPS for domains or HTTP for IP addresses only",
    "افتراضية" to "Default",
    "الإصدار" to "Version",
    "الإعدادات" to "Settings",
    "الاسم" to "Name",
    "التشخيص" to "Diagnostics",
    "الخادم" to "Server",
    "الطابعات" to "Printers",
    "العربية" to "Arabic",
    "العرض (mm)" to "Width (mm)",
    "العرض mm" to "Width mm",
    "العنوان" to "Address",
    "العنوان محظور أمنيًا" to "Address blocked for security",
    "اللغة" to "Language",
    "المظهر" to "Appearance",
    "تبويب جديد" to "New tab",
    "تحديث" to "Refresh",
    "تحديث الكاش" to "Clear cache",
    "تحسين الصور" to "Enhance images",
    "تصدير التشخيص" to "Export diagnostics",
    "تعذّر الاتصال" to "Connection failed",
    "تفعيل USB" to "Enable USB",
    "تقدم" to "Forward",
    "تلقائي" to "Auto",
    "تم الحفظ" to "Saved",
    "جاري الطباعة" to "Printing",
    "حذف الطابعة" to "Delete printer",
    "حفظ" to "Save",
    "خادم Odoo" to "Odoo server",
    "داكن" to "Dark",
    "درج النقدية" to "Cash drawer",
    "رابط غير صالح" to "Invalid URL",
    "رجوع" to "Back",
    "سيتم تسجيل خروجك من Odoo." to "You will be signed out of Odoo.",
    "ضبط" to "Configure",
    "فاتح" to "Light",
    "فشلت" to "Failed",
    "قص تلقائي" to "Auto cut",
    "لا توجد مهام طباعة" to "No print jobs",
    "متابعة" to "Continue",
    "متصل" to "Connected",
    "مركز التحكم" to "Control center",
    "مسح" to "Clear",
    "مسح المكتملة" to "Clear finished",
    "مسح بيانات التصفح" to "Clear browsing data",
    "مسح بيانات التصفح؟" to "Clear browsing data?",
    "مكتملة" to "Completed",
    "ملء الشاشة لنقطة البيع" to "Fullscreen POS",
    "ملغاة" to "Cancelled",
)

fun localize(text: String, language: String): String {
    if (language != "en") return text
    if (text.startsWith("السجل (")) return "Jobs (" + text.removePrefix("السجل (")
    if (text.startsWith("الطابعات (")) return "Printers (" + text.removePrefix("الطابعات (")
    return ENGLISH[text] ?: text
}

@Composable
fun tr(text: String): String = localize(text, LocalLanguage.current)
