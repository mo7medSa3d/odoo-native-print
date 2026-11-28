package com.company.odoonativeprint.protocol

import java.io.ByteArrayOutputStream
import java.io.DataOutputStream
import java.io.File
import java.io.InputStream
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

data class IppPrinterAttributes(
    val mediaSupported: List<String> = emptyList(),
    val sidesSupported: List<String> = emptyList(),
    val documentFormatsSupported: List<String> = emptyList(),
    val colorSupported: Boolean? = null,
    val printerName: String? = null,
)

/**
 * Pure-Kotlin RFC 8011 IPP 1.1 client (Get-Printer-Attributes + Print-Job).
 */
object IppClient {

    const val OP_PRINT_JOB: Int = 0x0002
    const val OP_GET_PRINTER_ATTRIBUTES: Int = 0x000B
    internal const val MAX_IPP_RESPONSE_BYTES: Int = 1024 * 1024

    private const val TAG_OPERATION_ATTRIBUTES = 0x01
    private const val TAG_JOB_ATTRIBUTES = 0x02
    private const val TAG_END_OF_ATTRIBUTES = 0x03
    private const val TAG_INTEGER = 0x21
    private const val TAG_TEXT_WITH_LANG = 0x35
    private const val TAG_NAME_WITH_LANG = 0x36
    private const val TAG_NAME_WITHOUT_LANG = 0x42
    private const val TAG_KEYWORD = 0x44
    private const val TAG_URI = 0x45
    private const val TAG_CHARSET = 0x47
    private const val TAG_NATURAL_LANG = 0x48
    private const val TAG_MIME_MEDIA_TYPE = 0x49

    fun normalizeHttpUrl(ippUri: String): String {
        val trimmed = ippUri.trim()
        return when {
            trimmed.startsWith("ipps://", ignoreCase = true) -> "https://" + trimmed.substring(7)
            trimmed.startsWith("ipp://", ignoreCase = true) -> "http://" + trimmed.substring(6)
            trimmed.startsWith("http://", ignoreCase = true) || trimmed.startsWith("https://", ignoreCase = true) -> trimmed
            else -> "http://$trimmed"
        }
    }

    fun buildGetPrinterAttributesRequest(printerUri: String, requestId: Int = 1): ByteArray {
        val baos = ByteArrayOutputStream(256)
        val out = DataOutputStream(baos)
        out.writeByte(1) // IPP 1.1
        out.writeByte(1)
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

    fun buildPrintJobHeader(
        printerUri: String,
        mimeType: String = "application/pdf",
        jobName: String = "Yaseir Print Job",
        copies: Int = 1,
        media: String? = null,
        sides: String? = null,
        requestId: Int = 1,
    ): ByteArray {
        val baos = ByteArrayOutputStream(512)
        val out = DataOutputStream(baos)
        out.writeByte(1) // IPP 1.1
        out.writeByte(1)
        out.writeShort(OP_PRINT_JOB)
        out.writeInt(requestId)

        out.writeByte(TAG_OPERATION_ATTRIBUTES)
        writeAttr(out, TAG_CHARSET, "attributes-charset", "utf-8")
        writeAttr(out, TAG_NATURAL_LANG, "attributes-natural-language", "en")
        writeAttr(out, TAG_URI, "printer-uri", printerUri)
        writeAttr(out, TAG_NAME_WITHOUT_LANG, "job-name", jobName.take(127))
        writeAttr(out, TAG_MIME_MEDIA_TYPE, "document-format", mimeType)

        if (copies > 1 || !media.isNullOrBlank() || !sides.isNullOrBlank()) {
            out.writeByte(TAG_JOB_ATTRIBUTES)
            if (copies > 1) writeIntAttr(out, "copies", copies.coerceIn(1, 99))
            if (!media.isNullOrBlank()) writeAttr(out, TAG_KEYWORD, "media", media)
            if (!sides.isNullOrBlank()) writeAttr(out, TAG_KEYWORD, "sides", sides)
        }

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
        val header = buildPrintJobHeader(
            printerUri = printerUri,
            mimeType = mimeType,
            jobName = jobName,
            copies = copies,
            media = media,
            sides = sides,
            requestId = requestId,
        )
        return ByteArray(header.size + documentBytes.size).also { combined ->
            System.arraycopy(header, 0, combined, 0, header.size)
            System.arraycopy(documentBytes, 0, combined, header.size, documentBytes.size)
        }
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

    fun parsePrinterAttributesResponse(bytes: ByteArray): IppPrinterAttributes {
        if (bytes.size < 9) return IppPrinterAttributes()
        var pos = 8
        var currentName = ""
        val media = mutableListOf<String>()
        val sides = mutableListOf<String>()
        val formats = mutableListOf<String>()
        var color: Boolean? = null
        var printerName: String? = null

        fun u16(offset: Int): Int =
            ((bytes[offset].toInt() and 0xFF) shl 8) or (bytes[offset + 1].toInt() and 0xFF)

        fun decodeTextValue(tag: Int, start: Int, len: Int): String {
            if (tag == TAG_TEXT_WITH_LANG || tag == TAG_NAME_WITH_LANG) {
                if (len < 4) return ""
                val langLen = u16(start)
                val textLenPos = start + 2 + langLen
                if (textLenPos + 2 > start + len) return ""
                val textLen = u16(textLenPos)
                val textStart = textLenPos + 2
                if (textStart + textLen > start + len) return ""
                return String(bytes, textStart, textLen, Charsets.UTF_8)
            }
            return String(bytes, start, len, Charsets.UTF_8)
        }

        while (pos < bytes.size) {
            val tag = bytes[pos++].toInt() and 0xFF
            if (tag == TAG_END_OF_ATTRIBUTES) break
            if (tag <= 0x0F) {
                currentName = ""
                continue
            }
            if (pos + 2 > bytes.size) break
            val nameLen = u16(pos)
            pos += 2
            if (pos + nameLen + 2 > bytes.size) break
            if (nameLen > 0) {
                currentName = String(bytes, pos, nameLen, Charsets.UTF_8)
            }
            pos += nameLen
            val valLen = u16(pos)
            pos += 2
            if (pos + valLen > bytes.size) break
            val valStart = pos
            pos += valLen

            if (tag in 0x10..0x1F) continue

            when (currentName) {
                "media-supported" -> decodeTextValue(tag, valStart, valLen).takeIf { it.isNotBlank() }?.let { media += it }
                "sides-supported" -> decodeTextValue(tag, valStart, valLen).takeIf { it.isNotBlank() }?.let { sides += it }
                "document-format-supported" -> decodeTextValue(tag, valStart, valLen).takeIf { it.isNotBlank() }?.let { formats += it }
                "color-supported" -> if (tag == 0x22 && valLen >= 1) color = bytes[valStart].toInt() != 0
                "printer-name" -> if (printerName == null) {
                    printerName = decodeTextValue(tag, valStart, valLen).takeIf { it.isNotBlank() }
                }
            }
        }
        return IppPrinterAttributes(
            mediaSupported = media.distinct(),
            sidesSupported = sides.distinct(),
            documentFormatsSupported = formats.distinct(),
            colorSupported = color,
            printerName = printerName,
        )
    }

    fun getPrinterAttributes(
        printerUri: String,
        connectTimeoutMs: Int = 2_500,
        readTimeoutMs: Int = 4_000,
    ): IppPrinterAttributes? {
        val payload = buildGetPrinterAttributesRequest(printerUri)
        val conn = (URL(normalizeHttpUrl(printerUri)).openConnection() as HttpURLConnection).apply {
            requestMethod = "POST"
            doOutput = true
            connectTimeout = connectTimeoutMs
            readTimeout = readTimeoutMs
            setRequestProperty("Content-Type", "application/ipp")
            setFixedLengthStreamingMode(payload.size)
        }
        return try {
            conn.outputStream.use { it.write(payload) }
            val code = conn.responseCode
            if (code !in 200..299) return null
            val bytes = conn.inputStream?.use { readCappedBytes(it) } ?: return null
            val hdr = if (bytes.size >= 8) parseResponseHeader(bytes, code) else return null
            if (!hdr.isSuccess) null else parsePrinterAttributesResponse(bytes)
        } catch (_: Exception) {
            null
        } finally {
            conn.disconnect()
        }
    }

    fun send(
        printerUri: String,
        ippPayload: ByteArray,
        connectTimeoutMs: Int = 5_000,
        readTimeoutMs: Int = 20_000,
    ): IppResponse {
        val conn = openPostConnection(printerUri, ippPayload.size.toLong(), connectTimeoutMs, readTimeoutMs)
        try {
            conn.outputStream.use { it.write(ippPayload) }
            return readIppResponse(conn)
        } finally {
            conn.disconnect()
        }
    }

    fun sendFile(
        printerUri: String,
        header: ByteArray,
        pdfFile: File,
        connectTimeoutMs: Int = 5_000,
        readTimeoutMs: Int = 20_000,
    ): IppResponse {
        val conn = openPostConnection(printerUri, header.size.toLong() + pdfFile.length(), connectTimeoutMs, readTimeoutMs)
        try {
            conn.outputStream.use { out ->
                out.write(header)
                pdfFile.inputStream().use { it.copyTo(out, bufferSize = 16 * 1024) }
                out.flush()
            }
            return readIppResponse(conn)
        } finally {
            conn.disconnect()
        }
    }

    private fun openPostConnection(
        printerUri: String,
        contentLength: Long,
        connectTimeoutMs: Int,
        readTimeoutMs: Int,
    ): HttpURLConnection = (URL(normalizeHttpUrl(printerUri)).openConnection() as HttpURLConnection).apply {
        requestMethod = "POST"
        doOutput = true
        connectTimeout = connectTimeoutMs
        readTimeout = readTimeoutMs
        setRequestProperty("Content-Type", "application/ipp")
        setFixedLengthStreamingMode(contentLength)
    }

    private fun readIppResponse(conn: HttpURLConnection): IppResponse {
        val code = conn.responseCode
        val contentType = conn.contentType?.lowercase().orEmpty()
        val stream = if (code in 200..299) conn.inputStream else conn.errorStream
        val responseBytes = stream?.use { readCappedBytes(it) } ?: ByteArray(0)
        val looksLikeIpp = responseBytes.size >= 8 &&
            (contentType.contains("application/ipp") ||
                (responseBytes[0].toInt() in 1..2 && responseBytes[1].toInt() in 0..9))
        return if (looksLikeIpp) {
            parseResponseHeader(responseBytes, code)
        } else {
            val fallbackStatus = if (code in 400..499) 0x0400 else 0x0500
            IppResponse(statusCode = fallbackStatus, requestId = 0, httpStatus = code)
        }
    }

    internal fun readCappedBytes(stream: InputStream, maxBytes: Int = MAX_IPP_RESPONSE_BYTES): ByteArray {
        val out = ByteArrayOutputStream()
        val buf = ByteArray(8192)
        var remaining = maxBytes
        while (remaining > 0) {
            val read = stream.read(buf, 0, minOf(buf.size, remaining))
            if (read <= 0) break
            out.write(buf, 0, read)
            remaining -= read
        }
        return out.toByteArray()
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
