package com.company.odoonativeprint.core

import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import com.company.odoonativeprint.BuildConfig
import java.security.MessageDigest

/**
 * Runtime APK signing certificate SHA-256 integrity guard (anti-repackaging).
 * Expected release certificate SHA-256 is stored split & XOR-masked (never plaintext).
 * Replace EXPECTED_CERT_PART_* with XOR(your_release_keystore_sha256, MASK_*) before release.
 */
object ApkIntegrityGuard {
    const val ERR_APK_TAMPERED = "ERR_LICENSE_REQUIRED"

    // Placeholder release cert SHA-256 (all-zero = unconfigured; replace before production release).
    private val CERT_MASK_A = byteArrayOf(
        0x41, 0x7c, 0x29, 0xe3.toByte(), 0x58, 0x1b, 0x94.toByte(), 0x6f,
        0x0d, 0xa2.toByte(), 0x3e, 0xc7.toByte(), 0x85.toByte(), 0x10, 0xf6.toByte(), 0x2b,
    )
    private val CERT_PART_A = byteArrayOf(
        0x41, 0x7c, 0x29, 0xe3.toByte(), 0x58, 0x1b, 0x94.toByte(), 0x6f,
        0x0d, 0xa2.toByte(), 0x3e, 0xc7.toByte(), 0x85.toByte(), 0x10, 0xf6.toByte(), 0x2b,
    )
    private val CERT_MASK_B = byteArrayOf(
        0x9a.toByte(), 0x53, 0x1e, 0x77, 0xd4.toByte(), 0x08, 0x62, 0xbf.toByte(),
        0x34, 0xe9.toByte(), 0x71, 0x05, 0xac.toByte(), 0x48, 0x2d, 0x83.toByte(),
    )
    private val CERT_PART_B = byteArrayOf(
        0x9a.toByte(), 0x53, 0x1e, 0x77, 0xd4.toByte(), 0x08, 0x62, 0xbf.toByte(),
        0x34, 0xe9.toByte(), 0x71, 0x05, 0xac.toByte(), 0x48, 0x2d, 0x83.toByte(),
    )

    fun unmaskDigest(partA: ByteArray, maskA: ByteArray, partB: ByteArray, maskB: ByteArray): ByteArray {
        val out = ByteArray(32)
        for (i in 0 until 16) {
            out[i] = (partA[i].toInt() xor maskA[i].toInt()).toByte()
            out[16 + i] = (partB[i].toInt() xor maskB[i].toInt()).toByte()
        }
        return out
    }

    /** Constant-time SHA-256 digest comparison (pure-JVM testable). */
    fun verifyCertDigest(actualSha256: ByteArray?, expectedSha256: ByteArray): Boolean {
        if (actualSha256 == null || actualSha256.size != 32 || expectedSha256.size != 32) return false
        var diff = 0
        for (i in 0 until 32) {
            diff = diff or (actualSha256[i].toInt() xor expectedSha256[i].toInt())
        }
        return diff == 0
    }

    fun isApkSignatureValid(context: Context): Boolean {
        if (BuildConfig.DEBUG) return true
        val expected = unmaskDigest(CERT_PART_A, CERT_MASK_A, CERT_PART_B, CERT_MASK_B)
        // If release cert digest has not been pinned yet (all zeros), skip only until owner pins it.
        if (expected.all { it == 0.toByte() }) return true
        val actual = readApkCertSha256(context)
        return verifyCertDigest(actual, expected)
    }

    @Suppress("DEPRECATION")
    private fun readApkCertSha256(context: Context): ByteArray? = try {
        val pm = context.packageManager
        val certBytes: ByteArray? = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            val pi = pm.getPackageInfo(context.packageName, PackageManager.GET_SIGNING_CERTIFICATES)
            val si = pi.signingInfo
            val sigs = if (si?.hasMultipleSigners() == true) si.apkContentsSigners else si?.signingCertificateHistory
            sigs?.firstOrNull()?.toByteArray()
        } else {
            val pi = pm.getPackageInfo(context.packageName, PackageManager.GET_SIGNATURES)
            pi.signatures?.firstOrNull()?.toByteArray()
        }
        certBytes?.let { MessageDigest.getInstance("SHA-256").digest(it) }
    } catch (_: Exception) {
        null
    }
}
