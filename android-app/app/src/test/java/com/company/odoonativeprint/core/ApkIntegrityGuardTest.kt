package com.company.odoonativeprint.core

import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test

class ApkIntegrityGuardTest {

    @Test
    fun `matching XOR-masked APK certificate SHA-256 succeeds`() {
        val realDigest = ByteArray(32) { i -> ((i * 17 + 5) and 0xFF).toByte() }
        val maskA = ByteArray(16) { i -> ((i * 31 + 3) and 0xFF).toByte() }
        val maskB = ByteArray(16) { i -> ((i * 23 + 9) and 0xFF).toByte() }
        val partA = ByteArray(16) { i -> (realDigest[i].toInt() xor maskA[i].toInt()).toByte() }
        val partB = ByteArray(16) { i -> (realDigest[16 + i].toInt() xor maskB[i].toInt()).toByte() }

        val expected = ApkIntegrityGuard.unmaskDigest(partA, maskA, partB, maskB)
        assertTrue(ApkIntegrityGuard.verifyCertDigest(realDigest, expected))
    }

    @Test
    fun `tampered or re-signed APK certificate SHA-256 is rejected`() {
        val expected = ByteArray(32) { i -> ((i * 13 + 7) and 0xFF).toByte() }
        val reSigned = expected.copyOf()
        reSigned[11] = (reSigned[11].toInt() xor 0x01).toByte() // 1-bit cert difference

        assertFalse(ApkIntegrityGuard.verifyCertDigest(reSigned, expected))
        assertFalse(ApkIntegrityGuard.verifyCertDigest(null, expected))
        assertFalse(ApkIntegrityGuard.verifyCertDigest(ByteArray(16), expected))
    }
}
