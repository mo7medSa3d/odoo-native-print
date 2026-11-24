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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileConfig {
    pub odoo_origin: String,
    #[serde(default)]
    pub auto_reprint: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedConfig {
    pub odoo_origin: Option<String>,
    pub locked_by_policy: bool,
    pub auto_reprint: bool,
    pub status_message: Option<String>,
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
    pub fn policy_locked() -> bool {
        false
    }
}

pub fn resolve_config(config_dir: &Path) -> ResolvedConfig {
    let policy_origin = gpo::policy_origin();
    let locked_by_policy = gpo::policy_locked() || policy_origin.is_some();
    let file = config_dir.join("config.json");
    let file_parsed = std::fs::read_to_string(&file).ok().map(|raw| {
        serde_json::from_str::<FileConfig>(&raw).map_err(|e| format!("config.json is invalid: {e}"))
    });
    let auto_reprint = match &file_parsed {
        Some(Ok(parsed)) => parsed.auto_reprint,
        _ => false,
    };
    let origin_result = if let Some(po) = policy_origin {
        validate_origin(&po)
    } else if let Some(parsed) = file_parsed {
        parsed.and_then(|cfg| validate_origin(&cfg.odoo_origin))
    } else if cfg!(debug_assertions) {
        match std::env::var("NATIVE_PRINT_ODOO_ORIGIN") {
            Ok(v) if !v.trim().is_empty() => validate_origin(&v),
            _ => Err(
                "no Odoo address configured; set odoo_origin (HTTPS, or HTTP to an IP address)"
                    .to_string(),
            ),
        }
    } else {
        Err(
            "no Odoo address configured; set odoo_origin (HTTPS, or HTTP to an IP address)"
                .to_string(),
        )
    };
    match origin_result {
        Ok(origin) => ResolvedConfig {
            odoo_origin: Some(origin),
            locked_by_policy,
            auto_reprint,
            status_message: None,
        },
        Err(msg) => ResolvedConfig {
            odoo_origin: None,
            locked_by_policy,
            auto_reprint,
            status_message: Some(msg),
        },
    }
}

pub fn save_origin(config_dir: &Path, origin: &str, auto_reprint: bool) -> Result<String, String> {
    if gpo::policy_locked() || gpo::policy_origin().is_some() {
        return Err("Odoo origin is locked by Windows Group Policy".to_string());
    }
    let validated = validate_origin(origin)?;
    let file = config_dir.join("config.json");
    let cfg = FileConfig {
        odoo_origin: validated.clone(),
        auto_reprint,
    };
    let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    atomic_write(&file, &json)?;
    Ok(validated)
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
        Ok(raw) => serde_json::from_str::<PrinterSettingsMap>(&raw)
            .map_err(|e| format!("printer_settings.json is unreadable: {e}")),
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
    fn printer_settings_checked_rejects_corrupt_json() {
        let dir = std::env::temp_dir().join(format!("np-pset-corrupt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(load_printer_settings_checked(&dir).unwrap().is_empty());
        std::fs::write(dir.join("printer_settings.json"), "{not-valid-json").unwrap();
        assert!(load_printer_settings_checked(&dir).is_err());
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
}
