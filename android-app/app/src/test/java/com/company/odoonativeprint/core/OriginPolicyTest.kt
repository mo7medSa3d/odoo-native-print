package com.company.odoonativeprint.core

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test

class OriginPolicyTest {

    @Test
    fun `accepts https hostnames and normalizes default port`() {
        assertEquals("https://erp.company.com", OriginPolicy.validateAndNormalize("https://erp.company.com:443/odoo").getOrThrow())
        assertEquals("https://erp.company.com:8443", OriginPolicy.validateAndNormalize("https://ERP.COMPANY.COM:8443/web").getOrThrow())
    }

    @Test
    fun `accepts http for ipv4 and ipv6 literals`() {
        assertEquals("http://192.168.1.50:8069", OriginPolicy.validateAndNormalize("http://192.168.1.50:8069/pos/ui").getOrThrow())
        assertEquals("http://127.0.0.1:8069", OriginPolicy.validateAndNormalize("http://127.0.0.1:8069").getOrThrow())
        assertEquals("http://[::1]:8069", OriginPolicy.validateAndNormalize("http://[::1]:8069").getOrThrow())
    }

    @Test
    fun `rejects http hostnames cloud metadata credentials and unsafe schemes`() {
        assertTrue(OriginPolicy.validateAndNormalize("http://erp.company.com").isFailure)
        assertTrue(OriginPolicy.validateAndNormalize("http://169.254.169.254/latest/meta-data").isFailure)
        assertTrue(OriginPolicy.validateAndNormalize("http://[::ffff:169.254.169.254]").isFailure)
        assertTrue(OriginPolicy.validateAndNormalize("https://user:pass@erp.company.com").isFailure)
        assertTrue(OriginPolicy.validateAndNormalize("javascript:alert(1)").isFailure)
        assertTrue(OriginPolicy.validateAndNormalize("file:///etc/passwd").isFailure)
    }

    @Test
    fun `isSameOrigin compares normalized origins accurately`() {
        assertTrue(OriginPolicy.isSameOrigin("https://erp.company.com", "https://erp.company.com:443/web#action=1"))
        assertFalse(OriginPolicy.isSameOrigin("https://erp.company.com", "https://evil.company.com/web"))
        assertFalse(OriginPolicy.isSameOrigin("https://erp.company.com", "http://192.168.1.50:8069"))
    }
}
