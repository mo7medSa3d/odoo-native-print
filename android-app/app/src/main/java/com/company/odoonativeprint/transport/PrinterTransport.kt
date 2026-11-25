package com.company.odoonativeprint.transport

import com.company.odoonativeprint.core.PrinterInfo
import com.company.odoonativeprint.core.PrinterSettings

sealed interface TransportResult {
    data object Ok : TransportResult
    data class Failure(
        val errorCode: String,
        val message: String,
        val retryable: Boolean = true,
    ) : TransportResult
}

interface RawByteTransport {
    fun sendRaw(
        printer: PrinterInfo,
        bytes: ByteArray,
        settings: PrinterSettings,
    ): TransportResult
}
