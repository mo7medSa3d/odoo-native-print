package com.company.odoonativeprint.worker

import com.company.odoonativeprint.core.PrinterProtocol
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test

class AndroidJobExecutorTest {

    @Test
    fun `passes raw label bytes when explicit mode is requested in AUTO protocol`() {
        assertTrue(
            AndroidJobExecutor.shouldPassRawLabelBytes(
                sourceKind = "text",
                settingsProtocol = PrinterProtocol.AUTO,
                resolvedProtocol = PrinterProtocol.ESCPOS,
                requestedMode = "zpl",
            ),
        )
        assertTrue(
            AndroidJobExecutor.shouldPassRawLabelBytes(
                sourceKind = "text",
                settingsProtocol = PrinterProtocol.ZPL,
                resolvedProtocol = PrinterProtocol.ZPL,
                requestedMode = "ZPL",
            ),
        )
        assertFalse(
            AndroidJobExecutor.shouldPassRawLabelBytes(
                sourceKind = "text",
                settingsProtocol = PrinterProtocol.AUTO,
                resolvedProtocol = PrinterProtocol.ESCPOS,
                requestedMode = "escpos",
            ),
        )
        assertFalse(
            AndroidJobExecutor.shouldPassRawLabelBytes(
                sourceKind = "pdf",
                settingsProtocol = PrinterProtocol.AUTO,
                resolvedProtocol = PrinterProtocol.ESCPOS,
                requestedMode = "zpl",
            ),
        )
    }
}
