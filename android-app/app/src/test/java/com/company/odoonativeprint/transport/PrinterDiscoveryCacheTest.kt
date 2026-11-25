package com.company.odoonativeprint.transport

import com.company.odoonativeprint.core.PrinterInfo
import com.company.odoonativeprint.core.PrinterTransportType
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test

class PrinterDiscoveryCacheTest {

    @Test
    fun `reuses 12s TTL cache and bypasses immediately on force refresh`() {
        var now = 1_000L
        var probes = 0
        val cache = PrinterDiscoveryCache(ttlMs = 12_000L, clock = { now }) {
            probes++
            listOf(PrinterInfo("P$probes", transport = PrinterTransportType.TCP_9100, address = "10.0.0.$probes:9100"))
        }

        val first = cache.get(force = false)
        val epochAfterFirst = cache.capsEpoch()
        now += 5_000L
        val second = cache.get(force = false)
        assertEquals(1, probes)
        assertEquals(first, second)
        assertEquals(epochAfterFirst, cache.capsEpoch())

        // Explicit force bypasses TTL and bumps capsEpoch
        val forced = cache.get(force = true)
        assertEquals(2, probes)
        assertEquals("P2", forced.single().name)
        assertTrue(cache.capsEpoch() > epochAfterFirst)

        // TTL expiry refreshes automatically
        now += 13_000L
        val expired = cache.get(force = false)
        assertEquals(3, probes)
        assertEquals("P3", expired.single().name)
    }
}
