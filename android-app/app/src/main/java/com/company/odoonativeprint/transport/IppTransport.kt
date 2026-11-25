package com.company.odoonativeprint.transport

import com.company.odoonativeprint.core.PrinterInfo
import com.company.odoonativeprint.protocol.IppClient
import java.io.IOException
import java.net.SocketTimeoutException

class IppTransport {
    fun sendDocument(
        printer: PrinterInfo,
        documentBytes: ByteArray,
        mimeType: String = "application/pdf",
        jobName: String = "Yaseir Print",
        copies: Int = 1,
        media: String? = null,
        sides: String? = null,
    ): TransportResult {
        return try {
            val payload = IppClient.buildPrintJobRequest(
                printerUri = printer.address,
                documentBytes = documentBytes,
                mimeType = mimeType,
                jobName = jobName,
                copies = copies,
                media = media,
                sides = sides,
            )
            val resp = IppClient.send(printer.address, payload)
            if (resp.isSuccess) {
                TransportResult.Ok
            } else {
                val retryable = resp.httpStatus >= 500 || resp.statusCode >= 0x0500
                TransportResult.Failure(
                    errorCode = "IPP_STATUS_0x${resp.statusCode.toString(16)}",
                    message = "IPP rejected job (HTTP ${resp.httpStatus}, IPP 0x${resp.statusCode.toString(16)})",
                    retryable = retryable,
                )
            }
        } catch (e: SocketTimeoutException) {
            TransportResult.Failure("IPP_TIMEOUT", e.message ?: "IPP request timed out", retryable = true)
        } catch (e: IOException) {
            TransportResult.Failure("IPP_IO_ERROR", e.message ?: "IPP network error", retryable = true)
        }
    }
}
