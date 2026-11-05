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
use std::path::Path;

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

pub fn validate_origin(origin: &str) -> Result<String, String> {
    let origin = origin.trim().trim_end_matches('/').to_string();
    if !print_core::security::origin_is_allowed(&origin, &origin) {
        return Err("invalid Odoo address (HTTPS, or HTTP to an IP address)".to_string());
    }
    if cfg!(not(debug_assertions)) {
        // Placeholder check by host labels, not substrings (audit
        // I3): the old `contains("example.")` also rejected real
        // domains like notexample.com and example.corp.com. Only the
        // RFC 2606 example domains (and their subdomains) and the
        // .invalid TLD count as placeholders.
        if let Some((host, _)) = print_core::security::origin_host_port(&origin) {
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
    Ok(origin)
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
        if st != ERROR_SUCCESS || cb < 2 {
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

pub fn load_origin(config_dir: &Path) -> Result<String, String> {
    if let Some(po) = gpo::policy_origin() {
        return validate_origin(&po);
    }
    let file = config_dir.join("config.json");
    if let Ok(raw) = std::fs::read_to_string(&file) {
        let parsed: FileConfig =
            serde_json::from_str(&raw).map_err(|e| format!("config.json is invalid: {e}"))?;
        return validate_origin(&parsed.odoo_origin);
    }
    if cfg!(debug_assertions) {
        if let Ok(v) = std::env::var("NATIVE_PRINT_ODOO_ORIGIN") {
            return validate_origin(&v);
        }
    }
    Err("no Odoo address configured; set odoo_origin (HTTPS, or HTTP to an IP address)".to_string())
}

pub fn resolve_config(config_dir: &Path) -> ResolvedConfig {
    let file = config_dir.join("config.json");
    let locked_by_policy = gpo::policy_locked() || gpo::policy_origin().is_some();
    let mut auto_reprint = false;
    if let Ok(raw) = std::fs::read_to_string(&file) {
        if let Ok(parsed) = serde_json::from_str::<FileConfig>(&raw) {
            auto_reprint = parsed.auto_reprint;
        }
    }
    match load_origin(config_dir) {
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
    std::fs::create_dir_all(config_dir).map_err(|e| e.to_string())?;
    let file = config_dir.join("config.json");
    let cfg = FileConfig {
        odoo_origin: validated.clone(),
        auto_reprint,
    };
    let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    // Write-temp-then-rename (audit I2): a crash or power loss
    // mid-write must not leave a truncated config.json that reads
    // as "no origin configured" on the next launch.
    let tmp = config_dir.join("config.json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &file).map_err(|e| e.to_string())?;
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

pub fn load_printer_settings(config_dir: &Path) -> PrinterSettingsMap {
    let file = config_dir.join("printer_settings.json");
    if let Ok(raw) = std::fs::read_to_string(&file) {
        if let Ok(parsed) = serde_json::from_str::<PrinterSettingsMap>(&raw) {
            return parsed;
        }
    }
    PrinterSettingsMap::new()
}

pub fn save_printer_settings(
    config_dir: &Path,
    settings: &PrinterSettingsMap,
) -> Result<(), String> {
    std::fs::create_dir_all(config_dir).map_err(|e| e.to_string())?;
    let file = config_dir.join("printer_settings.json");
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    // Same write-temp-then-rename discipline as save_origin (I2).
    let tmp = config_dir.join("printer_settings.json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &file).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn origin_validation() {
        assert!(validate_origin("http://192.168.1.20:8069").is_ok());
        assert!(validate_origin("http://192.168.1.20:8069/").is_ok());
        assert!(validate_origin("http://[::1]:8069").is_ok());
        assert!(validate_origin("https://192.168.1.20:8069").is_ok());
        assert!(validate_origin("https://odoo.company.local:8069").is_ok());
        assert!(validate_origin("http://8.8.8.8:8069").is_ok());
        assert!(validate_origin("http://68.221.27.248:8070").is_ok());
        assert!(validate_origin("http://odoo.company.local:8069").is_err());
        assert!(validate_origin("http://999.999.999.999:8069").is_err());
        assert!(validate_origin("not a url").is_err());
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
