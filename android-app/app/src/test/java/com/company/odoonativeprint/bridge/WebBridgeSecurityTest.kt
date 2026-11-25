package com.company.odoonativeprint.bridge

import com.company.odoonativeprint.core.AppOverview
import com.company.odoonativeprint.core.InMemoryJobPersistence
import com.company.odoonativeprint.core.JobQueueEngine
import com.company.odoonativeprint.core.PrintJobRecord
import com.company.odoonativeprint.core.PrinterCapabilities
import com.company.odoonativeprint.core.PrinterInfo
import com.company.odoonativeprint.core.QueueSummary
import org.json.JSONObject
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.io.TempDir
import java.io.File
import java.util.Base64

class WebBridgeSecurityTest {

    private fun buildDispatcher(tempDir: File): BridgeDispatcher {
        val queue = JobQueueEngine(InMemoryJobPersistence(), tempDir)
        val handler = object : RemoteBridgeHandler {
            override fun configuredOrigin(): String = "https://erp.company.com"
            override fun enqueuePdf(printer: String, bytes: ByteArray, copies: Int, idempotencyKey: String?, options: JSONObject): PrintJobRecord =
                queue.enqueue(printer, "pdf", bytes, copies, idempotencyKey = idempotencyKey)
            override fun enqueueText(printer: String, text: String, copies: Int, idempotencyKey: String?, options: JSONObject): PrintJobRecord =
                queue.enqueue(printer, "text", text.toByteArray(), copies, idempotencyKey = idempotencyKey)
            override fun enqueueImage(printer: String, bytes: ByteArray, copies: Int, idempotencyKey: String?, options: JSONObject): PrintJobRecord =
                queue.enqueue(printer, "image", bytes, copies, idempotencyKey = idempotencyKey)
            override fun printStatus(jobId: String): String? = queue.getJob(jobId)?.state?.wireName
            override fun jobDetail(jobId: String): PrintJobRecord? = queue.getJob(jobId)
            override fun appOverview(force: Boolean): AppOverview =
                AppOverview("1.5.7", "android", "https://erp.company.com", emptyList(), QueueSummary(), 1L)
            override fun listPrinters(force: Boolean): List<PrinterInfo> = emptyList()
            override fun printerForms(printer: String): List<String> = listOf("80mm Roll")
            override fun printerCapabilities(printer: String): PrinterCapabilities? = null
            override fun openControlCenter() = Unit
            override fun openSettings() = Unit
            override fun openTab(url: String) = Unit
            override fun reportEnvironment(report: String) = Unit
        }
        return BridgeDispatcher(handler, decodeBase64 = { Base64.getDecoder().decode(it) })
    }

    @Test
    fun `blocks subframe calls even on configured origin`(@TempDir tempDir: File) {
        val d = buildDispatcher(tempDir)
        val resp = d.dispatch("https://erp.company.com", isMainFrame = false, """{"id":"1","cmd":"list_printers","args":{}}""")
        assertFalse(resp.getBoolean("ok"))
        assertEquals("ERR_SUBFRAME_FORBIDDEN", resp.getString("code"))
    }

    @Test
    fun `blocks off-origin calls`(@TempDir tempDir: File) {
        val d = buildDispatcher(tempDir)
        val resp = d.dispatch("https://attacker.example", isMainFrame = true, """{"id":"2","cmd":"app_overview","args":{}}""")
        assertFalse(resp.getBoolean("ok"))
        assertEquals("ERR_ORIGIN_MISMATCH", resp.getString("code"))
    }

    @Test
    fun `blocks every admin-only command from remote Odoo pages`(@TempDir tempDir: File) {
        val d = buildDispatcher(tempDir)
        for (cmd in BridgeDispatcher.ADMIN_ONLY_COMMANDS) {
            val resp = d.dispatch("https://erp.company.com", isMainFrame = true, """{"id":"x","cmd":"$cmd","args":{}}""")
            assertFalse(resp.getBoolean("ok"), "Admin command $cmd must be rejected")
            assertEquals("ERR_ADMIN_LOCAL_ONLY", resp.getString("code"))
        }
    }

    @Test
    fun `allows valid print_text and blocks off-origin tab_open`(@TempDir tempDir: File) {
        val d = buildDispatcher(tempDir)
        val ok = d.dispatch(
            "https://erp.company.com",
            isMainFrame = true,
            """{"id":"3","cmd":"print_text","args":{"req":{"printer":"POS-1","text":"Receipt #1","copies":1}}}""",
        )
        assertTrue(ok.getBoolean("ok"))
        assertEquals("queued", ok.getJSONObject("result").getString("state"))

        val badTab = d.dispatch(
            "https://erp.company.com",
            isMainFrame = true,
            """{"id":"4","cmd":"tab_open","args":{"url":"https://evil.example/phish"}}""",
        )
        assertFalse(badTab.getBoolean("ok"))
        assertEquals("ERR_OFF_ORIGIN_TAB", badTab.getString("code"))
    }

    @Test
    fun `accepts Odoo printerPattern and resolves jobDetail by idempotencyKey`(@TempDir tempDir: File) {
        val d = buildDispatcher(tempDir)
        val enq = d.dispatch(
            "https://erp.company.com",
            isMainFrame = true,
            """{"id":"5","cmd":"print_text","args":{"req":{"printerPattern":"tcp://192.168.1.50:9100","idempotencyKey":"odoo-42","text":"Order #42","copies":1}}}""",
        )
        assertTrue(enq.getBoolean("ok"))
        assertEquals("tcp://192.168.1.50:9100", enq.getJSONObject("result").getString("printer"))

        val detail = d.dispatch(
            "https://erp.company.com",
            isMainFrame = true,
            """{"id":"6","cmd":"job_detail","args":{"jobId":"odoo-42"}}""",
        )
        assertTrue(detail.getBoolean("ok"))
        assertEquals("queued", detail.getJSONObject("result").getString("state"))
    }

    @Test
    fun `rejects oversized base64 image before invoking decoder`(@TempDir tempDir: File) {
        val store = InMemoryJobPersistence()
        val engine = JobQueueEngine(store, File(tempDir, "spool"))
        var decoderCalled = false
        val dispatcher = BridgeDispatcher(
            handler = object : RemoteBridgeHandler {
                override fun configuredOrigin(): String = "https://erp.company.com"
                override fun enqueuePdf(printer: String, bytes: ByteArray, copies: Int, idempotencyKey: String?, options: org.json.JSONObject) =
                    engine.enqueue(printer, "pdf", bytes, copies, idempotencyKey)
                override fun enqueueText(printer: String, text: String, copies: Int, idempotencyKey: String?, options: org.json.JSONObject) =
                    engine.enqueue(printer, "text", text.toByteArray(), copies, idempotencyKey)
                override fun enqueueImage(printer: String, bytes: ByteArray, copies: Int, idempotencyKey: String?, options: org.json.JSONObject) =
                    engine.enqueue(printer, "image", bytes, copies, idempotencyKey)
                override fun printStatus(jobId: String): String? = engine.getJob(jobId)?.state?.wireName
                override fun jobDetail(jobId: String) = engine.getJob(jobId)
                override fun appOverview(force: Boolean) = com.company.odoonativeprint.core.AppOverview("1.5.7", configuredOrigin(), 0, engine.summary())
                override fun listPrinters(force: Boolean) = emptyList<com.company.odoonativeprint.core.PrinterInfo>()
                override fun printerForms(printer: String) = emptyList<String>()
                override fun printerCapabilities(printer: String) = null
                override fun openControlCenter() {}
                override fun openSettings() {}
                override fun openTab(url: String) {}
                override fun reportEnvironment(report: String) {}
            },
            decodeBase64 = {
                decoderCalled = true
                ByteArray(0)
            },
        )
        val hugeB64 = "A".repeat((com.company.odoonativeprint.core.PayloadLimits.MAX_IMAGE_BYTES * 4) / 3 + 128)
        val res = dispatcher.dispatch(
            "https://erp.company.com",
            isMainFrame = true,
            """{"id":"7","cmd":"print_image","args":{"req":{"printer":"POS-1","bytes":"$hugeB64"}}}""",
        )
        assertFalse(res.getBoolean("ok"))
        assertFalse(decoderCalled, "Base64 decoder must not run on oversized payloads")
    }
}
