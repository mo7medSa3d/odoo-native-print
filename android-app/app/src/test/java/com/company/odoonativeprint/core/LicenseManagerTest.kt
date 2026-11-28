package com.company.odoonativeprint.core

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test

class LicenseManagerTest {

    // Deterministic test public key matching tools/license_keygen.py (never a production secret).
    private val testPubKey: ByteArray = hexToBytes(
        "197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61"
    )

    private val validKey =
        "ITBAA1.eyJjIjoiQWwtTm9vciBQT1MiLCJvIjoiaHR0cHM6Ly9lcnAuYWxub29yLnNhIiwiZXhwIjoxNzUyNTkyMDAwLCJ0IjoicHJvIn0.0lgZhgPPfyzkJKINBVP9ubl4oJbHx1qyulERer68nRqapJvnzRX8lmhYQ5y5yM7UCusBSwO5hdQ0W6GnJRuDAA"

    private val perpetualKey =
        "ITBAA1.eyJjIjoiRGVsdGEgUmV0YWlsIiwibyI6Imh0dHBzOi8vZXJwLmRlbHRhLmVnIiwiZXhwIjowLCJ0IjoicHJvIn0.JcxG0DFzvcNjS62oBHAjcI9zXSO7yDT5NJo73N4HxhxkYVHXFAGyRpFWq9BAyCNyEH5kIWSX1eZFx-5JqsjCBA"

    private val expiredKey =
        "ITBAA1.eyJjIjoiT2xkIFRyaWFsIiwibyI6Imh0dHBzOi8vZXJwLmFsbm9vci5zYSIsImV4cCI6MTcwMDA4NjQwMCwidCI6InRyaWFsIn0.GQ7iXkgmfjRY5TUCotjq3CuLZqdlEkTBB2vZ9eMPv6HY4dO0eeM8H4KC0r4HOBmS-ZkZ991oVcNIU9rJM0rYBg"

    private fun hexToBytes(hex: String): ByteArray =
        ByteArray(hex.length / 2) { i -> hex.substring(i * 2, i * 2 + 2).toInt(16).toByte() }

    @Test
    fun `valid pro and perpetual keys succeed`() {
        val r1 = LicenseManager.verify(
            key = validKey,
            configuredOrigin = "https://ERP.ALNOOR.SA:443/",
            nowUtc = 1750000100L,
            lastSeenUtc = 1750000000L,
            publicKey = testPubKey,
        )
        assertTrue(r1.isValid)
        val payload = (r1 as LicenseResult.Valid).payload
        assertEquals("Al-Noor POS", payload.customer)
        assertEquals("pro", payload.tier)
        assertFalse(payload.perpetual)
        assertEquals(30L, payload.daysRemaining(1750000100L))

        val r2 = LicenseManager.verify(
            key = perpetualKey,
            configuredOrigin = "https://erp.delta.eg",
            nowUtc = 2500000000L,
            lastSeenUtc = 2400000000L,
            publicKey = testPubKey,
        )
        assertTrue(r2.isValid)
        val perpetualPayload = (r2 as LicenseResult.Valid).payload
        assertTrue(perpetualPayload.perpetual)
        assertNull(perpetualPayload.daysRemaining(2500000000L))
    }

    @Test
    fun `daysRemaining handles perpetual, exact, fractional, and expired bounds`() {
        val perpetual = LicensePayload("Delta", "https://erp.delta.eg", exp = 0L, tier = "pro")
        assertNull(perpetual.daysRemaining(1_750_000_000L))

        val expiring = LicensePayload("Al-Noor", "https://erp.alnoor.sa", exp = 1_750_864_000L, tier = "trial")
        // Exact 10 days remaining
        assertEquals(10L, expiring.daysRemaining(1_750_000_000L))
        // 1 day + 1 second remaining rounds up to 2 days
        assertEquals(2L, expiring.daysRemaining(1_750_864_000L - 86_401L))
        // Exactly 1 day remaining
        assertEquals(1L, expiring.daysRemaining(1_750_864_000L - 86_400L))
        // 1 second remaining rounds up to 1 day
        assertEquals(1L, expiring.daysRemaining(1_750_864_000L - 1L))
        // Exact expiry and past expiry return 0
        assertEquals(0L, expiring.daysRemaining(1_750_864_000L))
        assertEquals(0L, expiring.daysRemaining(1_750_864_000L + 500L))
    }

    @Test
    fun `tampered key is rejected with ERR_LICENSE_REQUIRED`() {
        val chars = validKey.toCharArray()
        chars[14] = if (chars[14] == 'A') 'B' else 'A'
        val res = LicenseManager.verify(
            key = String(chars),
            configuredOrigin = "https://erp.alnoor.sa",
            nowUtc = 1750000100L,
            lastSeenUtc = 0L,
            publicKey = testPubKey,
        )
        assertFalse(res.isValid)
        val inv = res as LicenseResult.Invalid
        assertEquals(LicenseManager.ERR_LICENSE_REQUIRED, inv.code)
        assertEquals("invalid_signature", inv.reason)
    }

    @Test
    fun `wrong domain is rejected with ERR_LICENSE_REQUIRED`() {
        val res = LicenseManager.verify(
            key = validKey,
            configuredOrigin = "https://erp.other.sa",
            nowUtc = 1750000100L,
            lastSeenUtc = 0L,
            publicKey = testPubKey,
        )
        assertFalse(res.isValid)
        val inv = res as LicenseResult.Invalid
        assertEquals(LicenseManager.ERR_LICENSE_REQUIRED, inv.code)
        assertEquals("origin_mismatch", inv.reason)
    }

    @Test
    fun `expired key returns ERR_LICENSE_EXPIRED`() {
        val res = LicenseManager.verify(
            key = expiredKey,
            configuredOrigin = "https://erp.alnoor.sa",
            nowUtc = 1750000100L,
            lastSeenUtc = 0L,
            publicKey = testPubKey,
        )
        assertFalse(res.isValid)
        val inv = res as LicenseResult.Invalid
        assertEquals(LicenseManager.ERR_LICENSE_EXPIRED, inv.code)
    }

    @Test
    fun `clock rollback is rejected with ERR_LICENSE_REQUIRED`() {
        val res = LicenseManager.verify(
            key = validKey,
            configuredOrigin = "https://erp.alnoor.sa",
            nowUtc = 1750000000L,
            lastSeenUtc = 1750003600L,
            publicKey = testPubKey,
        )
        assertFalse(res.isValid)
        val inv = res as LicenseResult.Invalid
        assertEquals(LicenseManager.ERR_LICENSE_REQUIRED, inv.code)
        assertEquals("clock_rollback", inv.reason)
    }
}
