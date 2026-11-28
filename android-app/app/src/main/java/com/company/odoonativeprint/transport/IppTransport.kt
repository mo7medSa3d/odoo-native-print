package com.company.odoonativeprint.transport

import com.company.odoonativeprint.core.PrinterInfo
import com.company.odoonativeprint.protocol.IppClient
import com.company.odoonativeprint.protocol.IppResponse
import java.io.File
import java.io.IOException
import java.net.MalformedURLException
import java.net.SocketTimeoutException

class IppTransport {
    fun sendDocument(
        printer: PrinterInfo,
        documentBytes: ByteArray,
        mimeType: String = "application/pdf",
        jobName: String = "Itbaa",
        copies: Int = 1,
        media: String? = null,
        sides: String? = null,
    ): TransportResult = executeIpp {
        val payload = IppClient.buildPrintJobRequest(
            printerUri = printer.address,
            documentBytes = documentBytes,
            mimeType = mimeType,
            jobName = jobName,
            copies = copies,
            media = media,
            sides = sides,
        )
        IppClient.send(printer.address, payload)
    }

    fun sendDocument(
        printer: PrinterInfo,
        documentFile: File,
        mimeType: String = "application/pdf",
        jobName: String = "Itbaa",
        copies: Int = 1,
        media: String? = null,
        sides: String? = null,
    ): TransportResult = executeIpp {
        val header = IppClient.buildPrintJobHeader(
            printerUri = printer.address,
            mimeType = mimeType,
            jobName = jobName,
            copies = copies,
            media = media,
            sides = sides,
        )
        IppClient.sendFile(printer.address, header, documentFile)
    }

    private inline fun executeIpp(block: () -> IppResponse): TransportResult = try {
        val resp = block()
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
    } catch (e: MalformedURLException) {
        TransportResult.Failure("BAD_PRINTER_ADDRESS", e.message ?: "Invalid IPP URL", retryable = false)
    } catch (e: SocketTimeoutException) {
        TransportResult.Failure("IPP_TIMEOUT", e.message ?: "IPP request timed out", retryable = true)
    } catch (e: IOException) {
        TransportResult.Failure("IPP_IO_ERROR", e.message ?: "IPP network error", retryable = true)
    }
}
