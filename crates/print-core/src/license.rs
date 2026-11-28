//! Offline Ed25519 license verification for Itbaa (اطبع).
//!
//! Key format:
//!     ITBAA1.<Base64URL_no_pad(compact_json)>.<Base64URL_no_pad(ed25519_sig_64_bytes)>
//!
//! Payload JSON:
//!     {"c":"customer","o":"https://erp.domain.com","exp":1795737600,"t":"trial|pro"}
//!     (`exp: 0` = perpetual license)

use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub const LICENSE_PREFIX: &str = "ITBAA1";
pub const CLOCK_ROLLBACK_TOLERANCE_SEC: u64 = 300;
pub const ERR_LICENSE_REQUIRED: &str = "ERR_LICENSE_REQUIRED";
pub const ERR_LICENSE_EXPIRED: &str = "ERR_LICENSE_EXPIRED";

/// XOR-masked split Ed25519 public key halves — never stored as a contiguous raw 32-byte literal.
/// Replace with your production public key halves (generate via `python3 tools/license_keygen.py keygen`).
const PK_MASK_A: [u8; 16] = [
    0x5a, 0x3c, 0x91, 0xe4, 0x27, 0x8b, 0x6f, 0x12, 0xc3, 0x48, 0x7d, 0xa9, 0x35, 0xe1, 0x96, 0x04,
];
const PK_PART_A: [u8; 16] = [
    0x8d, 0x66, 0x09, 0xe5, 0xa5, 0x3a, 0x65, 0xa5, 0x16, 0x03, 0x83, 0x7a, 0xfc, 0x85, 0x91, 0x3e,
];
const PK_MASK_B: [u8; 16] = [
    0x8e, 0x19, 0x4b, 0xd2, 0x73, 0x50, 0xae, 0x61, 0x2c, 0xf5, 0x84, 0x37, 0x9b, 0x0d, 0x66, 0xc8,
];
const PK_PART_B: [u8; 16] = [
    0x80, 0xf8, 0x39, 0x21, 0xa9, 0xf6, 0x8d, 0x44, 0x83, 0xf7, 0x9e, 0x5f, 0x6c, 0x0a, 0x37, 0xd2,
];

/// Reassemble the 32-byte Ed25519 public key on the stack at verification time only.
#[inline(never)]
pub fn assemble_public_key() -> [u8; 32] {
    let mut out = [0u8; 32];
    for i in 0..16 {
        out[i] = PK_PART_A[i] ^ PK_MASK_A[i];
        out[16 + i] = PK_PART_B[i] ^ PK_MASK_B[i];
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LicensePayload {
    /// Customer / company name.
    pub c: String,
    /// Bound Odoo origin (e.g. "https://erp.domain.com").
    pub o: String,
    /// Expiration UTC unix timestamp (`0` = perpetual).
    pub exp: u64,
    /// Tier: "trial" or "pro".
    pub t: String,
}

impl LicensePayload {
    /// Whole days remaining until expiration (`None` for perpetual `exp == 0`).
    pub fn days_remaining(&self, now_utc: u64) -> Option<u64> {
        if self.exp == 0 {
            return None;
        }
        Some(self.exp.saturating_sub(now_utc) / 86_400)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LicenseError {
    Missing,
    Malformed,
    InvalidSignature,
    OriginMismatch,
    ClockRollback,
    Expired,
}

impl LicenseError {
    /// Stable error code returned to Odoo/UI callers on print gate rejection.
    pub fn code(&self) -> &'static str {
        match self {
            LicenseError::Expired => ERR_LICENSE_EXPIRED,
            _ => ERR_LICENSE_REQUIRED,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenseStatus {
    pub valid: bool,
    pub code: String,
    pub reason: String,
    pub customer: Option<String>,
    pub origin: Option<String>,
    pub tier: Option<String>,
    pub exp: Option<u64>,
    pub perpetual: bool,
    #[serde(default)]
    pub days_remaining: Option<u64>,
}

impl LicenseStatus {
    pub fn from_result(res: &Result<LicensePayload, LicenseError>) -> Self {
        match res {
            Ok(p) => Self {
                valid: true,
                code: "OK".to_string(),
                reason: "active".to_string(),
                customer: Some(p.c.clone()),
                origin: Some(p.o.clone()),
                tier: Some(p.t.clone()),
                exp: Some(p.exp),
                perpetual: p.exp == 0,
                days_remaining: p.days_remaining(now_utc_secs()),
            },
            Err(e) => Self {
                valid: false,
                code: e.code().to_string(),
                reason: format!("{e:?}"),
                customer: None,
                origin: None,
                tier: None,
                exp: None,
                perpetual: false,
                days_remaining: None,
            },
        }
    }
}

pub fn now_utc_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Normalize scheme + host + optional non-default port for exact origin matching.
pub fn normalize_origin(raw: &str) -> Option<String> {
    let s = raw.trim();
    let (scheme, rest) = s.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let (scheme_norm, host, port) =
        crate::security::parse_origin(&format!("{scheme}://{authority}"))?;
    match (scheme_norm, port) {
        ("https", 443) | ("http", 80) => Some(format!("{scheme_norm}://{host}")),
        _ => Some(format!("{scheme_norm}://{host}:{port}")),
    }
}

fn b64url_decode(input: &str) -> Option<Vec<u8>> {
    let stripped = input
        .strip_suffix("==")
        .or_else(|| input.strip_suffix('='))
        .unwrap_or(input);
    if stripped.len() != input.len() && input.len() % 4 != 0 {
        return None;
    }
    if stripped.is_empty() || stripped.len() % 4 == 1 {
        return None;
    }
    let bytes = stripped.as_bytes();
    let mut out = Vec::with_capacity((bytes.len() * 3) / 4);
    let mut buf: u32 = 0;
    let mut bits: u8 = 0;
    for &b in bytes {
        let val: u32 = match b {
            b'A'..=b'Z' => (b - b'A') as u32,
            b'a'..=b'z' => (b - b'a' + 26) as u32,
            b'0'..=b'9' => (b - b'0' + 52) as u32,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        buf = (buf << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    if bits > 0 && (buf & ((1u32 << bits) - 1)) != 0 {
        return None;
    }
    Some(out)
}

/// Verify an offline Ed25519 license using the embedded public key.
pub fn verify_license(
    key: Option<&str>,
    configured_origin: &str,
    now_utc: u64,
    last_seen_utc: u64,
) -> Result<LicensePayload, LicenseError> {
    let pk = assemble_public_key();
    verify_license_with_pubkey(key, &pk, configured_origin, now_utc, last_seen_utc)
}

/// Verify an offline Ed25519 license using an explicit 32-byte public key.
pub fn verify_license_with_pubkey(
    key: Option<&str>,
    public_key_bytes: &[u8; 32],
    configured_origin: &str,
    now_utc: u64,
    last_seen_utc: u64,
) -> Result<LicensePayload, LicenseError> {
    let raw = key
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(LicenseError::Missing)?;
    let mut parts = raw.split('.');
    let prefix = parts.next().ok_or(LicenseError::Malformed)?;
    let payload_b64 = parts.next().ok_or(LicenseError::Malformed)?;
    let sig_b64 = parts.next().ok_or(LicenseError::Malformed)?;
    if prefix != LICENSE_PREFIX || parts.next().is_some() {
        return Err(LicenseError::Malformed);
    }

    let payload_bytes = b64url_decode(payload_b64).ok_or(LicenseError::Malformed)?;
    let sig_bytes = b64url_decode(sig_b64).ok_or(LicenseError::Malformed)?;
    let sig_arr: [u8; 64] = sig_bytes.try_into().map_err(|_| LicenseError::Malformed)?;

    let vk =
        VerifyingKey::from_bytes(public_key_bytes).map_err(|_| LicenseError::InvalidSignature)?;
    let sig = Signature::from_bytes(&sig_arr);
    let signing_input = format!("{LICENSE_PREFIX}.{payload_b64}");
    vk.verify_strict(signing_input.as_bytes(), &sig)
        .map_err(|_| LicenseError::InvalidSignature)?;

    let payload: LicensePayload =
        serde_json::from_slice(&payload_bytes).map_err(|_| LicenseError::Malformed)?;
    if payload.c.trim().is_empty() || (payload.t != "trial" && payload.t != "pro") {
        return Err(LicenseError::Malformed);
    }

    let lic_origin = normalize_origin(&payload.o).ok_or(LicenseError::OriginMismatch)?;
    let exp_origin = normalize_origin(configured_origin).ok_or(LicenseError::OriginMismatch)?;
    if lic_origin != exp_origin {
        return Err(LicenseError::OriginMismatch);
    }

    if (payload.exp > 0 && now_utc == 0)
        || (last_seen_utc > 0
            && now_utc.saturating_add(CLOCK_ROLLBACK_TOLERANCE_SEC) < last_seen_utc)
    {
        return Err(LicenseError::ClockRollback);
    }

    if payload.exp > 0 && now_utc > payload.exp {
        return Err(LicenseError::Expired);
    }

    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn b64url_encode(data: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::with_capacity((data.len() * 4).div_ceil(3));
        let mut i = 0;
        while i < data.len() {
            let b0 = data[i] as u32;
            let b1 = if i + 1 < data.len() {
                data[i + 1] as u32
            } else {
                0
            };
            let b2 = if i + 2 < data.len() {
                data[i + 2] as u32
            } else {
                0
            };
            let n = (b0 << 16) | (b1 << 8) | b2;
            out.push(TABLE[((n >> 18) & 63) as usize] as char);
            out.push(TABLE[((n >> 12) & 63) as usize] as char);
            if i + 1 < data.len() {
                out.push(TABLE[((n >> 6) & 63) as usize] as char);
            }
            if i + 2 < data.len() {
                out.push(TABLE[(n & 63) as usize] as char);
            }
            i += 3;
        }
        out
    }

    fn make_signed(seed: [u8; 32], payload_json: &str) -> (String, [u8; 32]) {
        let sk = SigningKey::from_bytes(&seed);
        let pk = sk.verifying_key().to_bytes();
        let p_b64 = b64url_encode(payload_json.as_bytes());
        let msg = format!("{LICENSE_PREFIX}.{p_b64}");
        let sig = sk.sign(msg.as_bytes());
        let key = format!("{msg}.{}", b64url_encode(&sig.to_bytes()));
        (key, pk)
    }

    #[test]
    fn valid_pro_and_perpetual_keys() {
        let seed = [7u8; 32];
        let (key, pk) = make_signed(
            seed,
            r#"{"c":"Al-Noor Retail","o":"https://erp.alnoor.sa","exp":1800000000,"t":"pro"}"#,
        );
        let res = verify_license_with_pubkey(
            Some(&key),
            &pk,
            "https://ERP.ALNOOR.SA:443/",
            1750000000,
            1749999900,
        );
        assert!(res.is_ok());
        assert_eq!(res.unwrap().c, "Al-Noor Retail");

        // Perpetual (exp = 0)
        let (perp, pk2) = make_signed(
            seed,
            r#"{"c":"Delta Egypt","o":"https://erp.delta.eg","exp":0,"t":"pro"}"#,
        );
        assert!(verify_license_with_pubkey(
            Some(&perp),
            &pk2,
            "https://erp.delta.eg",
            2500000000,
            2400000000
        )
        .is_ok());
    }

    #[test]
    fn tampered_key_is_rejected() {
        let seed = [11u8; 32];
        let (key, pk) = make_signed(
            seed,
            r#"{"c":"Acme","o":"https://erp.acme.com","exp":1800000000,"t":"trial"}"#,
        );
        // Flip one character in payload
        let mut chars: Vec<char> = key.chars().collect();
        let idx = 12;
        chars[idx] = if chars[idx] == 'A' { 'B' } else { 'A' };
        let tampered: String = chars.into_iter().collect();
        let err =
            verify_license_with_pubkey(Some(&tampered), &pk, "https://erp.acme.com", 1750000000, 0)
                .unwrap_err();
        assert_eq!(err, LicenseError::InvalidSignature);
        assert_eq!(err.code(), ERR_LICENSE_REQUIRED);
    }

    #[test]
    fn wrong_domain_is_rejected() {
        let seed = [19u8; 32];
        let (key, pk) = make_signed(
            seed,
            r#"{"c":"Acme","o":"https://erp.acme.com","exp":1800000000,"t":"pro"}"#,
        );
        let err = verify_license_with_pubkey(
            Some(&key),
            &pk,
            "https://erp.other-company.com",
            1750000000,
            0,
        )
        .unwrap_err();
        assert_eq!(err, LicenseError::OriginMismatch);
        assert_eq!(err.code(), ERR_LICENSE_REQUIRED);
    }

    #[test]
    fn expired_key_returns_expired_error() {
        let seed = [23u8; 32];
        let (key, pk) = make_signed(
            seed,
            r#"{"c":"Acme","o":"https://erp.acme.com","exp":1700000000,"t":"trial"}"#,
        );
        let err =
            verify_license_with_pubkey(Some(&key), &pk, "https://erp.acme.com", 1700000001, 0)
                .unwrap_err();
        assert_eq!(err, LicenseError::Expired);
        assert_eq!(err.code(), ERR_LICENSE_EXPIRED);
    }

    #[test]
    fn clock_rollback_is_rejected() {
        let seed = [29u8; 32];
        let (key, pk) = make_signed(
            seed,
            r#"{"c":"Acme","o":"https://erp.acme.com","exp":1800000000,"t":"pro"}"#,
        );
        // Clock rolled back by 1 hour (> 300s tolerance)
        let err = verify_license_with_pubkey(
            Some(&key),
            &pk,
            "https://erp.acme.com",
            1750000000,
            1750003600,
        )
        .unwrap_err();
        assert_eq!(err, LicenseError::ClockRollback);
        assert_eq!(err.code(), ERR_LICENSE_REQUIRED);
    }

    #[test]
    fn missing_key_returns_required() {
        let err = verify_license_with_pubkey(
            None,
            &assemble_public_key(),
            "https://erp.acme.com",
            1750000000,
            0,
        )
        .unwrap_err();
        assert_eq!(err, LicenseError::Missing);
        assert_eq!(err.code(), ERR_LICENSE_REQUIRED);
    }

    #[test]
    fn single_byte_signature_tamper_is_rejected() {
        let seed = [31u8; 32];
        let (key, pk) = make_signed(
            seed,
            r#"{"c":"Acme","o":"https://erp.acme.com","exp":1800000000,"t":"pro"}"#,
        );
        let parts: Vec<&str> = key.split('.').collect();
        let mut sig = b64url_decode(parts[2]).unwrap();
        sig[17] ^= 0x01; // flip exactly one bit in the Ed25519 signature
        let bad_key = format!("{}.{}.{}", parts[0], parts[1], b64url_encode(&sig));
        let err =
            verify_license_with_pubkey(Some(&bad_key), &pk, "https://erp.acme.com", 1750000000, 0)
                .unwrap_err();
        assert_eq!(err, LicenseError::InvalidSignature);
        assert_eq!(err.code(), ERR_LICENSE_REQUIRED);
    }

    #[test]
    fn xor_masked_public_key_reassembles_cleanly() {
        let assembled = assemble_public_key();
        assert_eq!(assembled.len(), 32);
        assert_ne!(assembled, [0u8; 32]);
        assert!(VerifyingKey::from_bytes(&assembled).is_ok());
    }

    #[test]
    fn normalize_origin_rejects_malformed_and_canonicalizes_ipv6() {
        assert_eq!(
            normalize_origin("http://[0:0:0:0:0:0:0:1]:8069/web?db=1"),
            Some("http://[::1]:8069".to_string())
        );
        assert_eq!(
            normalize_origin("https://ERP.ACME.COM.:443/path#frag"),
            Some("https://erp.acme.com".to_string())
        );
        assert_eq!(normalize_origin("http://[::1]garbage"), None);
        assert_eq!(normalize_origin("http://[::1]:notaport"), None);
        assert_eq!(normalize_origin("https://[evil.com]:443"), None);
        assert_eq!(normalize_origin("https://erp.acme.com:0"), None);
        assert_eq!(normalize_origin("https://user:pass@erp.acme.com"), None);
    }

    #[test]
    fn b64url_rejects_trailing_garbage_and_noncanonical_bits() {
        let seed = [37u8; 32];
        let (key, pk) = make_signed(
            seed,
            r#"{"c":"Acme","o":"https://erp.acme.com","exp":1800000000,"t":"pro"}"#,
        );
        // Trailing garbage after '=' must not be ignored.
        let with_garbage = format!("{key}=GARBAGE");
        assert_eq!(
            verify_license_with_pubkey(
                Some(&with_garbage),
                &pk,
                "https://erp.acme.com",
                1750000000,
                0
            ),
            Err(LicenseError::Malformed)
        );
        // Non-zero unused trailing padding bits in 86-char signature must be rejected.
        let parts: Vec<&str> = key.split('.').collect();
        let mut sig_chars: Vec<char> = parts[2].chars().collect();
        let last = sig_chars.last_mut().unwrap();
        // Flip a low padding bit (64 bytes = 86 chars -> 4 trailing bits must be 0).
        *last = if *last == 'B' { 'C' } else { 'B' };
        let noncanonical = format!(
            "{}.{}.{}",
            parts[0],
            parts[1],
            sig_chars.into_iter().collect::<String>()
        );
        assert!(matches!(
            verify_license_with_pubkey(
                Some(&noncanonical),
                &pk,
                "https://erp.acme.com",
                1750000000,
                0
            ),
            Err(LicenseError::Malformed | LicenseError::InvalidSignature)
        ));
        assert!(b64url_decode("A").is_none());
        assert!(b64url_decode("").is_none());
        assert!(b64url_decode("AB=C").is_none());
    }

    #[test]
    fn zero_clock_rejects_expiring_license_but_allows_perpetual() {
        let seed = [41u8; 32];
        let (expiring, pk) = make_signed(
            seed,
            r#"{"c":"Acme","o":"https://erp.acme.com","exp":1800000000,"t":"pro"}"#,
        );
        assert_eq!(
            verify_license_with_pubkey(Some(&expiring), &pk, "https://erp.acme.com", 0, 0),
            Err(LicenseError::ClockRollback)
        );
        let (perp, pk2) = make_signed(
            seed,
            r#"{"c":"Acme","o":"https://erp.acme.com","exp":0,"t":"pro"}"#,
        );
        assert!(
            verify_license_with_pubkey(Some(&perp), &pk2, "https://erp.acme.com", 0, 0).is_ok()
        );
    }

    #[test]
    fn days_remaining_and_status_serde_compat() {
        let now = 1_750_000_000u64;
        let perp = LicensePayload {
            c: "Delta".into(),
            o: "https://erp.delta.eg".into(),
            exp: 0,
            t: "pro".into(),
        };
        assert_eq!(perp.days_remaining(now), None);

        let expiring = LicensePayload {
            c: "Al-Noor".into(),
            o: "https://erp.alnoor.sa".into(),
            exp: now + 30 * 86_400 + 43_200,
            t: "pro".into(),
        };
        assert_eq!(expiring.days_remaining(now), Some(30));
        assert_eq!(expiring.days_remaining(expiring.exp), Some(0));
        assert_eq!(expiring.days_remaining(expiring.exp + 500), Some(0));

        let status_perp = LicenseStatus::from_result(&Ok(perp));
        assert!(status_perp.perpetual);
        assert_eq!(status_perp.days_remaining, None);

        let status_err = LicenseStatus::from_result(&Err(LicenseError::Expired));
        assert!(!status_err.valid);
        assert_eq!(status_err.days_remaining, None);

        // Legacy JSON without `days_remaining` must deserialize cleanly.
        let legacy_json = r#"{"valid":true,"code":"OK","reason":"active","customer":"Acme","origin":"https://erp.acme.com","tier":"pro","exp":0,"perpetual":true}"#;
        let decoded: LicenseStatus = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(decoded.days_remaining, None);
    }

    #[test]
    fn verify_license_latency_under_1ms_per_op() {
        let seed = [43u8; 32];
        let (key, pk) = make_signed(
            seed,
            r#"{"c":"Latency Co","o":"https://erp.latency.example","exp":1900000000,"t":"pro"}"#,
        );
        const ITERS: u32 = 100;
        let start = std::time::Instant::now();
        for _ in 0..ITERS {
            let res = verify_license_with_pubkey(
                Some(&key),
                &pk,
                "https://erp.latency.example",
                1_750_000_000,
                1_749_999_900,
            );
            assert!(res.is_ok());
        }
        let elapsed = start.elapsed();
        let avg_ms = elapsed.as_secs_f64() * 1000.0 / f64::from(ITERS);
        eprintln!("verify_license_with_pubkey: {ITERS} ops in {elapsed:?} (avg {avg_ms:.3} ms/op)");
        // Debug builds run curve25519-dalek field ops unoptimized (~15 ms/op);
        // enforce the strict < 1 ms/op SLA in release builds and a generous
        // regression ceiling in debug builds.
        let max_avg_ms = if cfg!(debug_assertions) { 50.0 } else { 1.0 };
        assert!(
            avg_ms < max_avg_ms,
            "expected average verify latency < {max_avg_ms} ms/op, got {avg_ms:.3} ms/op ({elapsed:?} for {ITERS} ops)"
        );
    }
}
