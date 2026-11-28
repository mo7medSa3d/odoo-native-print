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
    "أيام" to "days",
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
    "الترخيص والتفعيل" to "License & activation",
    "التشخيص" to "Diagnostics",
    "الحالة" to "Status",
    "الخادم" to "Server",
    "الطابعات" to "Printers",
    "العربية" to "Arabic",
    "العرض (mm)" to "Width (mm)",
    "العرض mm" to "Width mm",
    "العميل" to "Customer",
    "العنوان" to "Address",
    "العنوان محظور أمنيًا" to "Address blocked for security",
    "اللغة" to "Language",
    "المتبقي" to "Remaining",
    "المظهر" to "Appearance",
    "تبويب جديد" to "New tab",
    "تجريبي" to "Trial",
    "تحديث" to "Refresh",
    "تحديث الكاش" to "Clear cache",
    "تحسين الصور" to "Enhance images",
    "تصدير التشخيص" to "Export diagnostics",
    "تعذّر الاتصال" to "Connection failed",
    "تفعيل USB" to "Enable USB",
    "تقدم" to "Forward",
    "تلقائي" to "Auto",
    "تم الحفظ" to "Saved",
    "تم تفعيل الرخصة بنجاح" to "License activated successfully",
    "جاري الطباعة" to "Printing",
    "حذف الطابعة" to "Delete printer",
    "حفظ" to "Save",
    "حفظ وتفعيل الرخصة" to "Save and activate license",
    "خادم Odoo" to "Odoo server",
    "دائم" to "Perpetual",
    "داكن" to "Dark",
    "درج النقدية" to "Cash drawer",
    "رابط غير صالح" to "Invalid URL",
    "رجوع" to "Back",
    "سيتم تسجيل خروجك من Odoo." to "You will be signed out of Odoo.",
    "ضبط" to "Configure",
    "غير مفعّل" to "Not activated",
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
    "مفتاح الترخيص منتهي الصلاحية" to "License key has expired",
    "مفتاح التفعيل (ITBAA1...)" to "License key (ITBAA1...)",
    "مفتاح غير صالح أو لا يطابق عنوان أودو" to "Invalid key or does not match Odoo origin",
    "مكتملة" to "Completed",
    "ملء الشاشة لنقطة البيع" to "Fullscreen POS",
    "ملغاة" to "Cancelled",
    "منتهي الصلاحية" to "Expired",
    "يوم" to "days",
    "يوم واحد" to "1 day",
)

fun localize(text: String, language: String): String {
    if (language != "en") return text
    if (text.startsWith("السجل (")) return "Jobs (" + text.removePrefix("السجل (")
    if (text.startsWith("الطابعات (")) return "Printers (" + text.removePrefix("الطابعات (")
    return ENGLISH[text] ?: text
}

@Composable
fun tr(text: String): String = localize(text, LocalLanguage.current)        "حول التطبيق والدعم" to "About & Support",
        "محرك طباعة صامتة ومباشرة لبيئات Odoo (17–20) مبني أصلياً بلغة Kotlin بدون IoT Box، بطابور SQLite WAL محلي وترخيص Ed25519 أوفلاين." to "Native silent printing engine for Odoo (17–20), built in Kotlin with zero IoT Box dependency, a local SQLite WAL queue, and offline Ed25519 licensing.",
        "واتساب الدعم" to "WhatsApp support",
        "البريد الإلكتروني" to "Email",
        "تواصل عبر واتساب" to "WhatsApp",
        "إرسال بريد" to "Email",
        "نسخ معلومات النظام" to "Copy system info",
        "تم نسخ معلومات النظام" to "System info copied",
        "تطوير وإشراف اطبع · جميع الحقوق محفوظة © 2026" to "Engineered by Itbaa · © 2026 All rights reserved",

