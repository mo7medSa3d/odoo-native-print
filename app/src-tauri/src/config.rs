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
    pub config_path: String,
    pub status_message: Option<String>,
}

pub fn validate_origin(origin: &str) -> Result<String, String> {
    let origin = origin.trim().trim_end_matches('/').to_string();
    if !print_core::security::origin_is_allowed(&origin, &origin) {
        return Err(format!(
            "configured Odoo origin is not a valid origin (HTTPS, or HTTP to a private IP): {origin}"
        ));
    }
    if cfg!(not(debug_assertions)) && (origin.contains(".invalid") || origin.contains("example.")) {
        return Err("configured Odoo origin is still a placeholder".to_string());
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
    Err(format!(
        "no Odoo origin configured: expected {} with {{\"odoo_origin\": \"https://...\"}} or an http:// private-IP origin",
        file.display()
    ))
}

pub fn resolve_config(config_dir: &Path) -> ResolvedConfig {
    let file = config_dir.join("config.json");
    let config_path = file.display().to_string();
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
            config_path,
            status_message: None,
        },
        Err(msg) => ResolvedConfig {
            odoo_origin: None,
            locked_by_policy,
            auto_reprint,
            config_path,
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
    std::fs::write(&file, json).map_err(|e| e.to_string())?;
    Ok(validated)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn origin_validation() {
        assert!(validate_origin("https://odoo.company.local:8069").is_ok());
        assert!(validate_origin("https://odoo.company.local:8069/").is_ok());
        assert!(validate_origin("http://odoo.company.local").is_err());
        // LAN by IP is allowed over HTTP (ADR-0002 amendment).
        assert!(validate_origin("http://192.168.1.20:8069").is_ok());
        assert!(validate_origin("http://192.168.1.20:8069/").is_ok());
        assert!(validate_origin("http://8.8.8.8:8069").is_err());
        assert!(validate_origin("not a url").is_err());
    }

    #[test]
    fn save_and_resolve_roundtrip() {
        let dir = std::env::temp_dir().join(format!("np-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let saved = save_origin(&dir, "https://erp.company.local:8443/", true).unwrap();
        assert_eq!(saved, "https://erp.company.local:8443");
        let resolved = resolve_config(&dir);
        assert_eq!(
            resolved.odoo_origin.as_deref(),
            Some("https://erp.company.local:8443")
        );
        assert!(resolved.auto_reprint);
        std::fs::remove_dir_all(&dir).ok();
    }
}
