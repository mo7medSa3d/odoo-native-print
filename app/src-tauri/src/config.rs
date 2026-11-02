//! App configuration: the single most security-sensitive value in the
//! product — the exact Odoo origin this install is locked to.
//!
//! Resolution order (first valid wins):
//! 1. `odoo_origin` in config.json in the app config directory
//!    (written by the MSI/GPO at install time; end users cannot edit a
//!    per-machine install's copy).
//! 2. Environment variable NATIVE_PRINT_ODOO_ORIGIN (development only;
//!    ignored in release builds).
//!
//! The app refuses to start without a valid HTTPS origin. Placeholder
//! hosts (.invalid, example.*) are rejected in release builds.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct FileConfig {
    odoo_origin: String,
}

pub fn validate_origin(origin: &str) -> Result<String, String> {
    let origin = origin.trim().trim_end_matches('/').to_string();
    if !print_core::security::origin_is_allowed(&origin, &origin) {
        return Err(format!(
            "configured Odoo origin is not a valid HTTPS origin: {origin}"
        ));
    }
    if cfg!(not(debug_assertions)) && (origin.contains(".invalid") || origin.contains("example.")) {
        return Err("configured Odoo origin is still a placeholder".to_string());
    }
    Ok(origin)
}

pub fn load_origin(config_dir: &std::path::Path) -> Result<String, String> {
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
        "no Odoo origin configured: expected {} with {{\"odoo_origin\": \"https://...\"}}",
        file.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn origin_validation() {
        assert!(validate_origin("https://odoo.company.local:8069").is_ok());
        assert!(validate_origin("https://odoo.company.local:8069/").is_ok());
        assert!(validate_origin("http://odoo.company.local").is_err());
        assert!(validate_origin("not a url").is_err());
    }
}
