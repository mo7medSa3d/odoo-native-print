package com.company.odoonativeprint.core

import java.net.URI
import java.util.Locale

/**
 * Mirrors desktop ADR-0002 origin security policy:
 * - HTTPS allowed for any valid host (except cloud metadata IP).
 * - HTTP allowed ONLY for IPv4/IPv6 literals (never hostnames), rejecting cloud metadata 169.254.169.254.
 * - Rejects embedded credentials (user:pass@) and non-http(s) schemes.
 */
object OriginPolicy {

    private val IPV4_REGEX = Regex("""^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$""")

    fun validateAndNormalize(raw: String): Result<String> {
        val trimmed = raw.trim()
        if (trimmed.isEmpty()) {
            return Result.failure(IllegalArgumentException("ORIGIN_EMPTY"))
        }
        val uri = try {
            URI(trimmed)
        } catch (e: Exception) {
            return Result.failure(IllegalArgumentException("ORIGIN_INVALID_URI"))
        }

        val scheme = uri.scheme?.lowercase(Locale.ROOT)
            ?: return Result.failure(IllegalArgumentException("ORIGIN_MISSING_SCHEME"))
        if (scheme != "https" && scheme != "http") {
            return Result.failure(IllegalArgumentException("ORIGIN_UNSUPPORTED_SCHEME"))
        }
        if (!uri.rawUserInfo.isNullOrEmpty()) {
            return Result.failure(IllegalArgumentException("ORIGIN_CREDENTIALS_FORBIDDEN"))
        }

        val rawHost = uri.host?.lowercase(Locale.ROOT)
            ?: return Result.failure(IllegalArgumentException("ORIGIN_MISSING_HOST"))
        if (rawHost.isBlank()) {
            return Result.failure(IllegalArgumentException("ORIGIN_MISSING_HOST"))
        }

        val unbracketed = rawHost.removePrefix("[").removeSuffix("]")
        val isIp = isValidIpv4(unbracketed) || isValidIpv6(unbracketed)

        if (isCloudMetadataAddress(unbracketed)) {
            return Result.failure(IllegalArgumentException("ORIGIN_METADATA_FORBIDDEN"))
        }
        if (scheme == "http" && !isIp) {
            return Result.failure(IllegalArgumentException("ORIGIN_HTTP_REQUIRES_IP"))
        }

        val port = uri.port
        if (port < -1 || port > 65535) {
            return Result.failure(IllegalArgumentException("ORIGIN_INVALID_PORT"))
        }

        val defaultPort = if (scheme == "https") 443 else 80
        val effectivePort = if (port == -1 || port == defaultPort) "" else ":$port"
        val formattedHost = if (unbracketed.contains(":") && !rawHost.startsWith("[")) {
            "[$unbracketed]"
        } else {
            rawHost
        }

        return Result.success("$scheme://$formattedHost$effectivePort")
    }

    fun isSameOrigin(configuredOrigin: String, candidateUrl: String): Boolean {
        val normConfig = validateAndNormalize(configuredOrigin).getOrNull() ?: return false
        val normCandidate = validateAndNormalize(candidateUrl).getOrNull() ?: return false
        return normConfig == normCandidate
    }

    internal fun isValidIpv4(host: String): Boolean {
        val m = IPV4_REGEX.matchEntire(host) ?: return false
        return (1..4).all { idx ->
            val part = m.groupValues[idx]
            val num = part.toIntOrNull() ?: return false
            num in 0..255 && (part.length == 1 || !part.startsWith("0"))
        }
    }

    internal fun isValidIpv6(host: String): Boolean {
        if (!host.contains(":")) return false
        // Reject invalid characters; accept standard hex/colon/ipv4-mapped notation
        if (!host.all { it in '0'..'9' || it in 'a'..'f' || it == ':' || it == '.' }) return false
        if (host.indexOf("::") != host.lastIndexOf("::")) return false
        return true
    }

    private fun isCloudMetadataAddress(unbracketed: String): Boolean {
        if (unbracketed == "169.254.169.254") return true
        // IPv4-mapped IPv6 forms: ::ffff:169.254.169.254 or ::ffff:a9fe:a9fe
        val lower = unbracketed.lowercase(Locale.ROOT)
        if (lower.endsWith(":169.254.169.254")) return true
        if (lower.endsWith(":a9fe:a9fe")) return true
        return false
    }
}
