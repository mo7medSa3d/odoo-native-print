package com.company.odoonativeprint.transport

import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import java.io.File

/** Source-level contract: Android System Print must not depend on a spool file
 * after print() returns, and must accept the image receipts POS actually sends. */
class AndroidSystemPrintContractTest {
    private fun source(relative: String): String {
        val tail = "app/src/main/java/com/company/odoonativeprint/$relative"
        return listOf(File(tail), File("../$tail"), File("../../$tail"))
            .first { it.isFile }
            .readText()
    }

    @Test
    fun `print adapter writes a captured byte array instead of reopening the spool file`() {
        val adapter = source("transport/AndroidPrintManagerAdapter.kt")
        assertTrue(adapter.contains("pdfBytes: ByteArray"))
        assertTrue(adapter.contains("output.write(snapshot)"))
        assertFalse(adapter.contains("FileInputStream"))
        assertFalse(adapter.contains("pdfFile"))
    }

    @Test
    fun `executor converts non PDF receipts to PDF for system print`() {
        val executor = source("worker/AndroidJobExecutor.kt")
        assertFalse(executor.contains("SYSTEM_PRINT_PDF_ONLY"))
        assertTrue(executor.contains("BitmapToPdfWriter.encode("))
        assertTrue(executor.contains("PrinterTransportType.ANDROID_SYSTEM"))
    }

    @Test
    fun `bitmap writer produces real PDF pages`() {
        val writer = source("render/BitmapToPdfWriter.kt")
        assertTrue(writer.contains("PdfDocument()"))
        assertTrue(writer.contains("startPage("))
        assertTrue(writer.contains("finishPage(page)"))
        assertTrue(writer.contains("document.writeTo(output)"))
    }
}
