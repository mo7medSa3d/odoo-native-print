//! App configuration: the single most security-sensitive value in the
//! product — the exact Odoo origin this install is locked to.
//!
//! Resolution order (first valid wins):
//! 1. Windows Registry Group Policy (`HKLM\Software\Policies\OdooNativePrint`
//!    value `OdooOrigin`, plus `DisableOriginChange`), matching
//!    `deploy/admx/OdooNativePrint.admx`.
//! 2. `odoo_origin` in `config.json` in the app config directory
//!    (written by the MSI/GPO at install time or via the local Control Center).
//! 3. Environment variable `NATIVE_PRINT_ODOO_ORIGIN` (development only;
//!    ignored in release builds).
//!
//! Placeholder hosts (`.invalid`, `example.*`) are rejected in release builds.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileConfig {
    #[serde(default)]
    pub odoo_origin: String,
    #[serde(default)]
    pub auto_reprint: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_blob: Option<String>,
    /// Legacy plaintext fields kept only for one-way migration into DPAPI `license_blob`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_key: Option<String>,
    #[serde(default)]
    pub last_seen_utc: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedConfig {
    pub odoo_origin: Option<String>,
    pub locked_by_policy: bool,
    pub auto_reprint: bool,
    pub license_key: Option<String>,
    pub license_locked_by_policy: bool,
    pub last_seen_utc: u64,
}

/// Write-temp-then-rename with unique temp filenames and a brief retry
/// on Windows sharing violations (e.g. antivirus or concurrent reader).
fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "invalid config path".to_string())?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("config.json");
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = parent.join(format!("{name}.{}.{seq}.tmp", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        if let Err(e) = f.write_all(content.as_bytes()).and_then(|()| f.sync_all()) {
            drop(f);
            let _ = std::fs::remove_file(&tmp);
            return Err(e.to_string());
        }
    }

    let mut last_err = None;
    for attempt in 0..5 {
        match std::fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                if attempt < 4 {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
            }
        }
    }
    let _ = std::fs::remove_file(&tmp);
    Err(last_err
        .map(|e| e.to_string())
        .unwrap_or_else(|| "failed to save config file".to_string()))
}

pub fn validate_origin(origin: &str) -> Result<String, String> {
    let trimmed = origin.trim().trim_end_matches('/');
    // If the admin/user pasted a full Odoo URL (e.g. https://erp.example.com/odoo
    // or http://192.168.1.20:8069/web/login) without userinfo, normalize it
    // down to its origin before validating.
    let candidate = match tauri::Url::parse(trimmed) {
        Ok(u) if u.username().is_empty() && u.password().is_none() => {
            let serialized = u.origin().ascii_serialization();
            if serialized == "null" {
                trimmed.to_string()
            } else {
                serialized
            }
        }
        _ => trimmed.to_string(),
    };

    if !print_core::security::origin_is_allowed(&candidate, &candidate) {
        return Err("invalid Odoo address (HTTPS, or HTTP to an IP address)".to_string());
    }
    if let Some((host, _)) = print_core::security::origin_host_port(&candidate) {
        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            if crate::commands::is_blocked_metadata_ip(ip) {
                return Err("cloud metadata address is not allowed".to_string());
            }
        } else if crate::commands::is_blocked_metadata_host(&host) {
            return Err("cloud metadata address is not allowed".to_string());
        }
    }
    if cfg!(not(debug_assertions)) {
        // Placeholder check by host labels, not substrings (audit
        // I3): the old `contains("example.")` also rejected real
        // domains like notexample.com and example.corp.com. Only the
        // RFC 2606 example domains (and their subdomains) and the
        // .invalid TLD count as placeholders.
        if let Some((host, _)) = print_core::security::origin_host_port(&candidate) {
            let h = host.to_ascii_lowercase();
            let placeholder = h == "invalid"
                || h.ends_with(".invalid")
                || ["example.com", "example.org", "example.net"]
                    .iter()
                    .any(|d| h == *d || h.ends_with(&format!(".{d}")));
            if placeholder {
                return Err("configured Odoo origin is still a placeholder".to_string());
            }
        }
    }
    Ok(candidate)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SealedLicenseState {
    #[serde(default)]
    license_key: Option<String>,
    #[serde(default)]
    last_seen_utc: u64,
}

#[cfg(windows)]
mod dpapi {
    use std::ffi::c_void;

    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            p_data_in: *const DataBlob,
            sz_data_descr: *const u16,
            p_optional_entropy: *const DataBlob,
            pv_reserved: *mut c_void,
            p_prompt_struct: *mut c_void,
            dw_flags: u32,
            p_data_out: *mut DataBlob,
        ) -> i32;

        fn CryptUnprotectData(
            p_data_in: *const DataBlob,
            ppsz_data_descr: *mut *mut u16,
            p_optional_entropy: *const DataBlob,
            pv_reserved: *mut c_void,
            p_prompt_struct: *mut c_void,
            dw_flags: u32,
            p_data_out: *mut DataBlob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(hmem: *mut c_void) -> *mut c_void;
    }

    const ENTROPY: &[u8] = b"Itbaa.License.DPAPI.v1";

    pub fn protect(plain: &[u8]) -> Option<Vec<u8>> {
        let in_blob = DataBlob {
            cb_data: plain.len() as u32,
            pb_data: plain.as_ptr() as *mut u8,
        };
        let ent = DataBlob {
            cb_data: ENTROPY.len() as u32,
            pb_data: ENTROPY.as_ptr() as *mut u8,
        };
        let mut out_blob = DataBlob {
            cb_data: 0,
            pb_data: std::ptr::null_mut(),
        };
        let ok = unsafe {
            CryptProtectData(
                &in_blob,
                std::ptr::null(),
                &ent,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out_blob,
            )
        };
        if ok == 0 || out_blob.pb_data.is_null() {
            return None;
        }
        let slice =
            unsafe { std::slice::from_raw_parts(out_blob.pb_data, out_blob.cb_data as usize) };
        let vec = slice.to_vec();
        unsafe { LocalFree(out_blob.pb_data as *mut c_void) };
        Some(vec)
    }

    pub fn unprotect(cipher: &[u8]) -> Option<Vec<u8>> {
        let in_blob = DataBlob {
            cb_data: cipher.len() as u32,
            pb_data: cipher.as_ptr() as *mut u8,
        };
        let ent = DataBlob {
            cb_data: ENTROPY.len() as u32,
            pb_data: ENTROPY.as_ptr() as *mut u8,
        };
        let mut out_blob = DataBlob {
            cb_data: 0,
            pb_data: std::ptr::null_mut(),
        };
        let ok = unsafe {
            CryptUnprotectData(
                &in_blob,
                std::ptr::null_mut(),
                &ent,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out_blob,
            )
        };
        if ok == 0 || out_blob.pb_data.is_null() {
            return None;
        }
        let slice =
            unsafe { std::slice::from_raw_parts(out_blob.pb_data, out_blob.cb_data as usize) };
        let vec = slice.to_vec();
        unsafe { LocalFree(out_blob.pb_data as *mut c_void) };
        Some(vec)
    }
}

#[cfg(not(windows))]
mod dpapi {
    // Non-Windows fallback (unit tests on Linux): simple reversible XOR envelope; production runs on Windows DPAPI.
    const K: &[u8] = b"ItbaaNonWinTestKey";
    pub fn protect(plain: &[u8]) -> Option<Vec<u8>> {
        Some(
            plain
                .iter()
                .enumerate()
                .map(|(i, b)| b ^ K[i % K.len()])
                .collect(),
        )
    }
    pub fn unprotect(cipher: &[u8]) -> Option<Vec<u8>> {
        protect(cipher)
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let bytes = s.trim().as_bytes();
    if bytes.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        let hex_str = std::str::from_utf8(chunk).ok()?;
        out.push(u8::from_str_radix(hex_str, 16).ok()?);
    }
    Some(out)
}

fn seal_license_state(key: Option<String>, last_seen_utc: u64) -> Option<String> {
    let json = serde_json::to_vec(&SealedLicenseState {
        license_key: key,
        last_seen_utc,
    })
    .ok()?;
    let protected = dpapi::protect(&json)?;
    Some(format!("dpapi1:{}", hex_encode(&protected)))
}

fn unseal_license_state(cfg: &FileConfig) -> SealedLicenseState {
    if let Some(blob) = cfg
        .license_blob
        .as_deref()
        .and_then(|b| b.strip_prefix("dpapi1:"))
    {
        if let Some(raw) = hex_decode(blob).and_then(|c| dpapi::unprotect(&c)) {
            if let Ok(state) = serde_json::from_slice::<SealedLicenseState>(&raw) {
                return state;
            }
        }
    }
    SealedLicenseState {
        license_key: cfg.license_key.clone(),
        last_seen_utc: cfg.last_seen_utc,
    }
}

#[cfg(windows)]
mod gpo {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;

    type Hkey = *mut c_void;
    const HKEY_LOCAL_MACHINE: Hkey = 0x8000_0002usize as Hkey;
    const RRF_RT_REG_SZ: u32 = 0x0000_0002;
    const RRF_RT_REG_DWORD: u32 = 0x0000_0010;
    const ERROR_SUCCESS: i32 = 0;

    #[link(name = "advapi32")]
    extern "system" {
        fn RegGetValueW(
            hkey: Hkey,
            sub_key: *const u16,
            value: *const u16,
            flags: u32,
            kind: *mut u32,
            data: *mut c_void,
            cb_data: *mut u32,
        ) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    pub fn policy_origin() -> Option<String> {
        let sub = wide("Software\\Policies\\OdooNativePrint");
        let val = wide("OdooOrigin");
        let mut cb: u32 = 0;
        let st = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                sub.as_ptr(),
                val.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut cb,
            )
        };
        if st != ERROR_SUCCESS || cb < 2 || cb > 8192 {
            return None;
        }
        let mut buf = vec![0u16; (cb as usize).div_ceil(2)];
        let st2 = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                sub.as_ptr(),
                val.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut c_void,
                &mut cb,
            )
        };
        if st2 != ERROR_SUCCESS {
            return None;
        }
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let s = String::from_utf16_lossy(&buf[..end]).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }

    pub fn policy_license_key() -> Option<String> {
        let sub = wide("Software\\Policies\\OdooNativePrint");
        let val = wide("LicenseKey");
        let mut cb: u32 = 0;
        let st = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                sub.as_ptr(),
                val.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut cb,
            )
        };
        if st != ERROR_SUCCESS || cb < 2 || cb > 8192 {
            return None;
        }
        let mut buf = vec![0u16; (cb as usize).div_ceil(2)];
        let st2 = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                sub.as_ptr(),
                val.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut c_void,
                &mut cb,
            )
        };
        if st2 != ERROR_SUCCESS {
            return None;
        }
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let s = String::from_utf16_lossy(&buf[..end]).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }

    pub fn policy_locked() -> bool {
        let sub = wide("Software\\Policies\\OdooNativePrint");
        let val = wide("DisableOriginChange");
        let mut dword: u32 = 0;
        let mut cb = std::mem::size_of::<u32>() as u32;
        let st = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                sub.as_ptr(),
                val.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                &mut dword as *mut u32 as *mut c_void,
                &mut cb,
            )
        };
        st == ERROR_SUCCESS && dword != 0
    }
}

#[cfg(not(windows))]
mod gpo {
    pub fn policy_origin() -> Option<String> {
        None
    }
    pub fn policy_license_key() -> Option<String> {
        None
    }
    pub fn policy_locked() -> bool {
        false
    }
}

#[cfg(windows)]
fn machine_config_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("PROGRAMDATA")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\ProgramData"));
    Some(base.join("OdooNativePrint").join("config.json"))
}

#[cfg(not(windows))]
fn machine_config_path() -> Option<std::path::PathBuf> {
    None
}

fn read_file_config(path: &Path) -> Option<Result<FileConfig, String>> {
    std::fs::read_to_string(path).ok().map(|raw| {
        serde_json::from_str::<FileConfig>(&raw).map_err(|e| format!("config.json is invalid: {e}"))
    })
}

fn resolve_config_with(
    config_dir: &Path,
    policy_origin: Option<String>,
    policy_locked: bool,
    machine_config: Option<&Path>,
) -> ResolvedConfig {
    let locked_by_policy = policy_locked || policy_origin.is_some();
    // When locked by Group Policy, never trust user-writable %APPDATA% config.json
    // for either odoo_origin or auto_reprint; only read machine-level install config.
    let file_parsed = if locked_by_policy {
        machine_config.and_then(read_file_config)
    } else {
        read_file_config(&config_dir.join("config.json"))
    };
    let auto_reprint = match &file_parsed {
        Some(Ok(parsed)) => parsed.auto_reprint,
        _ => false,
    };
    let user_cfg = read_file_config(&config_dir.join("config.json")).and_then(|r| r.ok());
    let policy_lic = gpo::policy_license_key();
    let license_locked_by_policy = policy_lic.is_some();
    let sealed_user = user_cfg
        .as_ref()
        .map(unseal_license_state)
        .unwrap_or_default();
    let license_key = policy_lic.or_else(|| match &file_parsed {
        Some(Ok(parsed)) => unseal_license_state(parsed)
            .license_key
            .or_else(|| sealed_user.license_key.clone()),
        _ => sealed_user.license_key.clone(),
    });
    let last_seen_utc = sealed_user.last_seen_utc;
    let origin_result = if let Some(po) = policy_origin {
        validate_origin(&po)
    } else if let Some(parsed) = file_parsed {
        parsed.and_then(|cfg| validate_origin(&cfg.odoo_origin))
    } else if !locked_by_policy && cfg!(debug_assertions) {
        match std::env::var("NATIVE_PRINT_ODOO_ORIGIN") {
            Ok(v) if !v.trim().is_empty() => validate_origin(&v),
            _ => Err("no Odoo address configured".to_string()),
        }
    } else {
        Err("no Odoo address configured".to_string())
    };
    ResolvedConfig {
        odoo_origin: origin_result.ok(),
        locked_by_policy,
        auto_reprint,
        license_key,
        license_locked_by_policy,
        last_seen_utc,
    }
}

pub fn resolve_config(config_dir: &Path) -> ResolvedConfig {
    let machine_path = machine_config_path();
    resolve_config_with(
        config_dir,
        gpo::policy_origin(),
        gpo::policy_locked(),
        machine_path.as_deref(),
    )
}

pub fn save_origin(config_dir: &Path, origin: &str, auto_reprint: bool) -> Result<String, String> {
    if gpo::policy_locked() || gpo::policy_origin().is_some() {
        return Err("Odoo origin is locked by Windows Group Policy".to_string());
    }
    let validated = validate_origin(origin)?;
    let file = config_dir.join("config.json");
    let mut cfg = read_file_config(&file)
        .and_then(|r| r.ok())
        .unwrap_or_default();
    cfg.odoo_origin = validated.clone();
    cfg.auto_reprint = auto_reprint;
    let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    atomic_write(&file, &json)?;
    Ok(validated)
}

pub fn save_license_key(config_dir: &Path, key: &str) -> Result<(), String> {
    if gpo::policy_license_key().is_some() {
        return Err("License key is locked by Windows Group Policy".to_string());
    }
    let file = config_dir.join("config.json");
    let mut cfg = read_file_config(&file)
        .and_then(|r| r.ok())
        .unwrap_or_default();
    let prev = unseal_license_state(&cfg);
    let trimmed = key.trim();
    let next_key = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    };
    let sealed = seal_license_state(next_key, prev.last_seen_utc)
        .ok_or_else(|| "failed to seal license state".to_string())?;
    cfg.license_blob = Some(sealed);
    cfg.license_key = None;
    cfg.last_seen_utc = 0;
    let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    atomic_write(&file, &json)
}

pub fn persist_last_seen_utc(config_dir: &Path, now_utc: u64) {
    let file = config_dir.join("config.json");
    let mut cfg = read_file_config(&file)
        .and_then(|r| r.ok())
        .unwrap_or_default();
    let prev = unseal_license_state(&cfg);
    if now_utc <= prev.last_seen_utc {
        return;
    }
    let Some(sealed) = seal_license_state(prev.license_key, now_utc) else {
        return;
    };
    cfg.license_blob = Some(sealed);
    cfg.license_key = None;
    cfg.last_seen_utc = 0;
    if let Ok(json) = serde_json::to_string_pretty(&cfg) {
        let _ = atomic_write(&file, &json);
    }
}

/// Per-printer defaults chosen in the Control Center (owner brief,
/// 2026-11-05): the paper, tray, duplex and color a printer uses
/// when a print job does not specify them itself. They live in
/// their own file next to config.json — save_origin rewrites
/// config.json wholesale and must not swallow them.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PrinterSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tray: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplex: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<bool>,
}

pub type PrinterSettingsMap = std::collections::BTreeMap<String, PrinterSettings>;

pub fn load_printer_settings_checked(config_dir: &Path) -> Result<PrinterSettingsMap, String> {
    let file = config_dir.join("printer_settings.json");
    match std::fs::read_to_string(&file) {
        Ok(raw) => match serde_json::from_str::<PrinterSettingsMap>(&raw) {
            Ok(map) => Ok(map),
            Err(e) => {
                let backup = config_dir.join("printer_settings.json.corrupt");
                let _ = std::fs::remove_file(&backup);
                let _ = std::fs::rename(&file, &backup);
                eprintln!(
                    "Itbaa: printer_settings.json was corrupt ({e}); moved to {}",
                    backup.display()
                );
                Ok(PrinterSettingsMap::new())
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(PrinterSettingsMap::new()),
        Err(e) => Err(format!("cannot read printer_settings.json: {e}")),
    }
}

pub fn load_printer_settings(config_dir: &Path) -> PrinterSettingsMap {
    load_printer_settings_checked(config_dir).unwrap_or_default()
}

pub fn save_printer_settings(
    config_dir: &Path,
    settings: &PrinterSettingsMap,
) -> Result<(), String> {
    let file = config_dir.join("printer_settings.json");
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    atomic_write(&file, &json)
}

pub fn is_restorable_url(origin: &str, url: &str) -> bool {
    let Ok(parsed) = tauri::Url::parse(url.trim()) else {
        return false;
    };
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return false;
    }
    let parsed_origin = parsed.origin().ascii_serialization();
    if !print_core::security::origin_is_allowed(origin, &parsed_origin) {
        return false;
    }
    let path = parsed.path();
    !path.starts_with("/web/login") && !path.starts_with("/web/session/logout")
}

pub fn load_last_url(data_dir: &Path, origin: &str) -> Option<String> {
    let raw = std::fs::read_to_string(data_dir.join("last_url.txt")).ok()?;
    let trimmed = raw.trim();
    if is_restorable_url(origin, trimmed) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

pub fn save_last_url(data_dir: &Path, origin: &str, url: &str) {
    let trimmed = url.trim();
    if !is_restorable_url(origin, trimmed) {
        return;
    }
    let path = data_dir.join("last_url.txt");
    if std::fs::read_to_string(&path)
        .ok()
        .as_deref()
        .map(str::trim)
        == Some(trimmed)
    {
        return;
    }
    let _ = atomic_write(&path, trimmed);
}

pub fn clear_last_url(data_dir: &Path) {
    let _ = std::fs::remove_file(data_dir.join("last_url.txt"));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn origin_validation() {
        assert!(validate_origin("http://192.168.1.20:8069").is_ok());
        assert!(validate_origin("http://192.168.1.20:8069/").is_ok());
        assert!(validate_origin("http://192.168.1.20:8069/web/login").is_ok());
        assert!(validate_origin("https://erp.company.local/odoo").is_ok());
        assert!(validate_origin("http://[::1]:8069").is_ok());
        assert!(validate_origin("https://192.168.1.20:8069").is_ok());
        assert!(validate_origin("https://odoo.company.local:8069").is_ok());
        assert!(validate_origin("http://8.8.8.8:8069").is_ok());
        assert!(validate_origin("http://203.0.113.10:8070").is_ok());
        assert!(validate_origin("http://odoo.company.local:8069").is_err());
        assert!(validate_origin("http://999.999.999.999:8069").is_err());
        assert!(validate_origin("https://metadata.google.internal").is_err());
        assert!(validate_origin("http://169.254.169.254").is_err());
        assert!(validate_origin("not a url").is_err());
    }

    #[test]
    fn printer_settings_checked_recovers_corrupt_json() {
        let dir = std::env::temp_dir().join(format!("np-pset-corrupt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(load_printer_settings_checked(&dir).unwrap().is_empty());
        std::fs::write(dir.join("printer_settings.json"), "{not-valid-json").unwrap();
        assert!(load_printer_settings_checked(&dir).unwrap().is_empty());
        assert!(dir.join("printer_settings.json.corrupt").exists());
        assert!(!dir.join("printer_settings.json").exists());
        save_printer_settings(&dir, &PrinterSettingsMap::new()).unwrap();
        assert!(load_printer_settings_checked(&dir).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn policy_locked_ignores_user_config_and_uses_machine_config() {
        let dir = std::env::temp_dir().join(format!("np-gpo-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let user_dir = dir.join("user");
        let machine_dir = dir.join("machine");
        std::fs::create_dir_all(&user_dir).unwrap();
        std::fs::create_dir_all(&machine_dir).unwrap();
        std::fs::write(
            user_dir.join("config.json"),
            r#"{"odoo_origin":"https://user.corp.local","auto_reprint":true}"#,
        )
        .unwrap();
        let machine_file = machine_dir.join("config.json");
        // Locked with no machine config: user config must not supply origin or auto_reprint.
        let r1 = resolve_config_with(&user_dir, None, true, Some(&machine_file));
        assert!(r1.locked_by_policy);
        assert_eq!(r1.odoo_origin, None);
        assert!(!r1.auto_reprint);
        // Locked with GPO origin: user config must not enable auto_reprint.
        let r2 = resolve_config_with(
            &user_dir,
            Some("https://gpo.corp.local".to_string()),
            false,
            Some(&machine_file),
        );
        assert!(r2.locked_by_policy);
        assert_eq!(r2.odoo_origin.as_deref(), Some("https://gpo.corp.local"));
        assert!(!r2.auto_reprint);
        // Locked with machine config present: machine config is used.
        std::fs::write(
            &machine_file,
            r#"{"odoo_origin":"https://machine.corp.local","auto_reprint":true}"#,
        )
        .unwrap();
        let r3 = resolve_config_with(&user_dir, None, true, Some(&machine_file));
        assert!(r3.locked_by_policy);
        assert_eq!(
            r3.odoo_origin.as_deref(),
            Some("https://machine.corp.local")
        );
        assert!(r3.auto_reprint);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn printer_settings_roundtrip() {
        let dir = std::env::temp_dir().join(format!("np-pset-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Missing file reads as an empty map, never an error.
        assert!(load_printer_settings(&dir).is_empty());
        let mut map = PrinterSettingsMap::new();
        map.insert(
            "HP LaserJet".to_string(),
            PrinterSettings {
                paper: Some("A4".to_string()),
                tray: Some("Tray 2".to_string()),
                duplex: Some(true),
                color: None,
            },
        );
        save_printer_settings(&dir, &map).unwrap();
        let back = load_printer_settings(&dir);
        assert_eq!(back, map);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_and_resolve_roundtrip() {
        let dir = std::env::temp_dir().join(format!("np-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let saved = save_origin(&dir, "http://192.168.1.20:8069/", true).unwrap();
        assert_eq!(saved, "http://192.168.1.20:8069");
        let resolved = resolve_config(&dir);
        assert_eq!(
            resolved.odoo_origin.as_deref(),
            Some("http://192.168.1.20:8069")
        );
        assert!(resolved.auto_reprint);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn last_url_roundtrip_and_guards() {
        let dir = std::env::temp_dir().join(format!("np-lasturl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let origin = "http://127.0.0.1:8069";
        save_last_url(&dir, origin, "http://127.0.0.1:8069/pos/ui?config_id=1");
        assert_eq!(
            load_last_url(&dir, origin).as_deref(),
            Some("http://127.0.0.1:8069/pos/ui?config_id=1")
        );
        // Login/logout and off-origin URLs must not overwrite or restore.
        save_last_url(&dir, origin, "http://127.0.0.1:8069/web/login");
        save_last_url(&dir, origin, "https://evil.example.com/pos/ui");
        assert_eq!(
            load_last_url(&dir, origin).as_deref(),
            Some("http://127.0.0.1:8069/pos/ui?config_id=1")
        );
        clear_last_url(&dir);
        assert_eq!(load_last_url(&dir, origin), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
