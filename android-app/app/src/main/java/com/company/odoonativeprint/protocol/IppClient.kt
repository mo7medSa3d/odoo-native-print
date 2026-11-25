package com.company.odoonativeprint.protocol

import java.io.ByteArrayOutputStream
import java.io.DataOutputStream
import java.net.HttpURLConnection
import java.net.URL

data class IppResponse(
    val statusCode: Int,
    val requestId: Int,
    val httpStatus: Int,
) {
    val isSuccess: Boolean
        get() = httpStatus in 200..299 && statusCode in 0x0000..0x00FF
}

/**
 * Pure-Kotlin RFC 8011 IPP 2.0 client (Get-Printer-Attributes + Print-Job).
 */
object IppClient {

    const val OP_PRINT_JOB: Int = 0x0002
    const val OP_GET_PRINTER_ATTRIBUTES: Int = 0x000B

    private const val TAG_OPERATION_ATTRIBUTES = 0x01
    private const val TAG_JOB_ATTRIBUTES = 0x02
    private const val TAG_END_OF_ATTRIBUTES = 0x03
    private const val TAG_INTEGER = 0x21
    private const val TAG_NAME_WITHOUT_LANG = 0x42
    private const val TAG_KEYWORD = 0x44
    private const val TAG_URI = 0x45
    private const val TAG_CHARSET = 0x47
    private const val TAG_NATURAL_LANG = 0x48
    private const val TAG_MIME_MEDIA_TYPE = 0x49

    fun normalizeHttpUrl(ippUri: String): String = when {
        ippUri.startsWith("ipps://", ignoreCase = true) -> "https://" + ippUri.substring(7)
        ippUri.startsWith("ipp://", ignoreCase = true) -> "http://" + ippUri.substring(6)
        else -> ippUri
    }

    fun buildGetPrinterAttributesRequest(printerUri: String, requestId: Int = 1): ByteArray {
        val baos = ByteArrayOutputStream(256)
        val out = DataOutputStream(baos)
        out.writeByte(2) // IPP 2.0
        out.writeByte(0)
        out.writeShort(OP_GET_PRINTER_ATTRIBUTES)
        out.writeInt(requestId)
        out.writeByte(TAG_OPERATION_ATTRIBUTES)
        writeAttr(out, TAG_CHARSET, "attributes-charset", "utf-8")
        writeAttr(out, TAG_NATURAL_LANG, "attributes-natural-language", "en")
        writeAttr(out, TAG_URI, "printer-uri", printerUri)
        out.writeByte(TAG_END_OF_ATTRIBUTES)
        out.flush()
        return baos.toByteArray()
    }

    fun buildPrintJobRequest(
        printerUri: String,
        documentBytes: ByteArray,
        mimeType: String = "application/pdf",
        jobName: String = "Yaseir Print Job",
        copies: Int = 1,
        media: String? = null,
        sides: String? = null,
        requestId: Int = 1,
    ): ByteArray {
        val baos = ByteArrayOutputStream(documentBytes.size + 512)
        val out = DataOutputStream(baos)
        out.writeByte(2)
        out.writeByte(0)
        out.writeShort(OP_PRINT_JOB)
        out.writeInt(requestId)

        out.writeByte(TAG_OPERATION_ATTRIBUTES)
        writeAttr(out, TAG_CHARSET, "attributes-charset", "utf-8")
        writeAttr(out, TAG_NATURAL_LANG, "attributes-natural-language", "en")
        writeAttr(out, TAG_URI, "printer-uri", printerUri)
        writeAttr(out, TAG_NAME_WITHOUT_LANG, "job-name", jobName.take(127))
        writeAttr(out, TAG_MIME_MEDIA_TYPE, "document-format", mimeType)

        if (copies > 1 || media != null || sides != null) {
            out.writeByte(TAG_JOB_ATTRIBUTES)
            if (copies > 1) writeIntAttr(out, "copies", copies.coerceIn(1, 99))
            if (!media.isNullOrBlank()) writeAttr(out, TAG_KEYWORD, "media", media)
            if (!sides.isNullOrBlank()) writeAttr(out, TAG_KEYWORD, "sides", sides)
        }

        out.writeByte(TAG_END_OF_ATTRIBUTES)
        out.write(documentBytes)
        out.flush()
        return baos.toByteArray()
    }

    fun parseResponseHeader(bytes: ByteArray, httpStatus: Int): IppResponse {
        require(bytes.size >= 8) { "IPP_RESPONSE_TOO_SHORT" }
        val statusCode = ((bytes[2].toInt() and 0xFF) shl 8) or (bytes[3].toInt() and 0xFF)
        val requestId = ((bytes[4].toInt() and 0xFF) shl 24) or
            ((bytes[5].toInt() and 0xFF) shl 16) or
            ((bytes[6].toInt() and 0xFF) shl 8) or
            (bytes[7].toInt() and 0xFF)
        return IppResponse(statusCode = statusCode, requestId = requestId, httpStatus = httpStatus)
    }

    fun send(
        printerUri: String,
        ippPayload: ByteArray,
        connectTimeoutMs: Int = 5_000,
        readTimeoutMs: Int = 20_000,
    ): IppResponse {
        val httpUrl = normalizeHttpUrl(printerUri)
        val conn = (URL(httpUrl).openConnection() as HttpURLConnection).apply {
            requestMethod = "POST"
            doOutput = true
            connectTimeout = connectTimeoutMs
            readTimeout = readTimeoutMs
            setRequestProperty("Content-Type", "application/ipp")
            setFixedLengthStreamingMode(ippPayload.size)
        }
        try {
            conn.outputStream.use { it.write(ippPayload) }
            val code = conn.responseCode
            val stream = if (code in 200..299) conn.inputStream else conn.errorStream
            val responseBytes = stream?.use { it.readBytes() } ?: ByteArray(0)
            return if (responseBytes.size >= 8) {
                parseResponseHeader(responseBytes, code)
            } else {
                IppResponse(statusCode = 0x0500, requestId = 0, httpStatus = code)
            }
        } finally {
            conn.disconnect()
        }
    }

    private fun writeAttr(out: DataOutputStream, tag: Int, name: String, value: String) {
        val nameBytes = name.toByteArray(Charsets.UTF_8)
        val valBytes = value.toByteArray(Charsets.UTF_8)
        out.writeByte(tag)
        out.writeShort(nameBytes.size)
        out.write(nameBytes)
        out.writeShort(valBytes.size)
        out.write(valBytes)
    }

    private fun writeIntAttr(out: DataOutputStream, name: String, value: Int) {
        val nameBytes = name.toByteArray(Charsets.UTF_8)
        out.writeByte(TAG_INTEGER)
        out.writeShort(nameBytes.size)
        out.write(nameBytes)
        out.writeShort(4)
        out.writeInt(value)
    }
}
