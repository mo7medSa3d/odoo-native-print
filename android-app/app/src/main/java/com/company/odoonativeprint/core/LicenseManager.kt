package com.company.odoonativeprint.core

import org.json.JSONObject
import java.net.URI
import java.nio.charset.StandardCharsets
import java.util.Base64

data class LicensePayload(
    val customer: String,
    val origin: String,
    val exp: Long,
    val tier: String,
) {
    val perpetual: Boolean get() = exp == 0L

    /** Whole days remaining until expiry (rounded up while valid), or null for perpetual (`exp == 0L`). */
    fun daysRemaining(nowUtc: Long = LicenseManager.nowUtcSecs()): Long? {
        if (perpetual) return null
        val remainingSecs = exp - nowUtc
        if (remainingSecs <= 0L) return 0L
        return ((remainingSecs - 1L) / 86_400L) + 1L
    }
}

sealed class LicenseResult {
    data class Valid(val payload: LicensePayload) : LicenseResult()
    data class Invalid(val code: String, val reason: String) : LicenseResult()

    val isValid: Boolean get() = this is Valid
}

object LicenseManager {
    const val PREFIX = "ITBAA1"
    const val CLOCK_ROLLBACK_TOLERANCE_SEC = 300L
    const val ERR_LICENSE_REQUIRED = "ERR_LICENSE_REQUIRED"
    const val ERR_LICENSE_EXPIRED = "ERR_LICENSE_EXPIRED"

    /**
     * Embedded 32-byte Ed25519 public key (replace with your production public key
     * generated via `python3 tools/license_keygen.py keygen`; never commit the private key).
     */
    private val PK_MASK_A = byteArrayOf(0x5a, 0x3c, 0x91.toByte(), 0xe4.toByte(), 0x27, 0x8b.toByte(), 0x6f, 0x12, 0xc3.toByte(), 0x48, 0x7d, 0xa9.toByte(), 0x35, 0xe1.toByte(), 0x96.toByte(), 0x04)
    private val PK_PART_A = byteArrayOf(0x8d.toByte(), 0x66, 0x09, 0xe5.toByte(), 0xa5.toByte(), 0x3a, 0x65, 0xa5.toByte(), 0x16, 0x03, 0x83.toByte(), 0x7a, 0xfc.toByte(), 0x85.toByte(), 0x91.toByte(), 0x3e)
    private val PK_MASK_B = byteArrayOf(0x8e.toByte(), 0x19, 0x4b, 0xd2.toByte(), 0x73, 0x50, 0xae.toByte(), 0x61, 0x2c, 0xf5.toByte(), 0x84.toByte(), 0x37, 0x9b.toByte(), 0x0d, 0x66, 0xc8.toByte())
    private val PK_PART_B = byteArrayOf(0x80.toByte(), 0xf8.toByte(), 0x39, 0x21, 0xa9.toByte(), 0xf6.toByte(), 0x8d.toByte(), 0x44, 0x83.toByte(), 0xf7.toByte(), 0x9e.toByte(), 0x5f, 0x6c, 0x0a, 0x37, 0xd2.toByte())

    fun assemblePublicKey(): ByteArray {
        val out = ByteArray(32)
        for (i in 0 until 16) {
            out[i] = (PK_PART_A[i].toInt() xor PK_MASK_A[i].toInt()).toByte()
            out[16 + i] = (PK_PART_B[i].toInt() xor PK_MASK_B[i].toInt()).toByte()
        }
        return out
    }

    fun nowUtcSecs(): Long = System.currentTimeMillis() / 1000L

    fun normalizeOrigin(raw: String?): String? {
        val trimmed = raw?.trim()?.trimEnd('/') ?: return null
        if (trimmed.isEmpty()) return null
        return try {
            val uri = URI(trimmed)
            val scheme = uri.scheme?.lowercase() ?: return null
            if (scheme != "http" && scheme != "https") return null
            val host = uri.host?.lowercase() ?: return null
            val port = uri.port
            val effectivePort = when {
                scheme == "https" && port == 443 -> -1
                scheme == "http" && port == 80 -> -1
                else -> port
            }
            if (effectivePort > 0) "$scheme://$host:$effectivePort" else "$scheme://$host"
        } catch (_: Exception) {
            null
        }
    }

    private fun b64UrlDecode(text: String): ByteArray? = try {
        Base64.getUrlDecoder().decode(text)
    } catch (_: Exception) {
        null
    }

    fun verify(
        key: String?,
        configuredOrigin: String?,
        nowUtc: Long = nowUtcSecs(),
        lastSeenUtc: Long = 0L,
        publicKey: ByteArray = assemblePublicKey(),
    ): LicenseResult {
        val raw = key?.trim().orEmpty()
        if (raw.isEmpty()) {
            return LicenseResult.Invalid(ERR_LICENSE_REQUIRED, "missing")
        }
        val parts = raw.split('.')
        if (parts.size != 3 || parts[0] != PREFIX) {
            return LicenseResult.Invalid(ERR_LICENSE_REQUIRED, "malformed")
        }
        val payloadB64 = parts[1]
        val sigB64 = parts[2]
        val payloadBytes = b64UrlDecode(payloadB64)
            ?: return LicenseResult.Invalid(ERR_LICENSE_REQUIRED, "malformed_payload")
        val sigBytes = b64UrlDecode(sigB64)
            ?: return LicenseResult.Invalid(ERR_LICENSE_REQUIRED, "malformed_sig")
        if (sigBytes.size != 64) {
            return LicenseResult.Invalid(ERR_LICENSE_REQUIRED, "sig_length")
        }

        val signingInput = "$PREFIX.$payloadB64".toByteArray(StandardCharsets.US_ASCII)
        if (!Ed25519Verifier.verify(publicKey, signingInput, sigBytes)) {
            return LicenseResult.Invalid(ERR_LICENSE_REQUIRED, "invalid_signature")
        }

        val payload = try {
            val obj = JSONObject(String(payloadBytes, StandardCharsets.UTF_8))
            val c = obj.getString("c").trim()
            val o = obj.getString("o").trim()
            val exp = obj.getLong("exp")
            val t = obj.getString("t").trim()
            if (c.isEmpty() || (t != "trial" && t != "pro")) {
                return LicenseResult.Invalid(ERR_LICENSE_REQUIRED, "invalid_claims")
            }
            LicensePayload(customer = c, origin = o, exp = exp, tier = t)
        } catch (_: Exception) {
            return LicenseResult.Invalid(ERR_LICENSE_REQUIRED, "invalid_json")
        }

        val licOrigin = normalizeOrigin(payload.origin)
        val expOrigin = normalizeOrigin(configuredOrigin)
        if (licOrigin == null || expOrigin == null || licOrigin != expOrigin) {
            return LicenseResult.Invalid(ERR_LICENSE_REQUIRED, "origin_mismatch")
        }

        if (lastSeenUtc > 0L && nowUtc + CLOCK_ROLLBACK_TOLERANCE_SEC < lastSeenUtc) {
            return LicenseResult.Invalid(ERR_LICENSE_REQUIRED, "clock_rollback")
        }

        if (payload.exp > 0L && nowUtc > payload.exp) {
            return LicenseResult.Invalid(ERR_LICENSE_EXPIRED, "expired")
        }

        return LicenseResult.Valid(payload)
    }
}
