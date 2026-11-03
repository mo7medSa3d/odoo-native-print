use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One printer model's known quirks (spec section 5: clipping offsets,
/// dithering, rotation). Keyed in the database by driver/model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrinterQuirks {
    /// Content shift needed to compensate for this model's clipping, mm.
    pub offset_x_mm: f64,
    pub offset_y_mm: f64,
    /// 180-degree rotation needed by some label printers.
    pub rotate_180: bool,
    /// Force 1-bit threshold rendering (thermal/barcode safety).
    pub force_threshold: bool,
    pub notes: String,
}

impl Default for PrinterQuirks {
    fn default() -> Self {
        Self {
            offset_x_mm: 0.0,
            offset_y_mm: 0.0,
            rotate_180: false,
            force_threshold: false,
            notes: String::new(),
        }
    }
}

/// Versioned JSON profile database. Admin overrides from Odoo are merged
/// on top of the shipped database (override wins per key).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileDb {
    pub version: u32,
    pub by_driver: HashMap<String, PrinterQuirks>,
}

impl ProfileDb {
    pub fn new() -> Self {
        Self {
            version: 1,
            by_driver: HashMap::new(),
        }
    }
    /// Case-insensitive substring match: a key matches when the printer's
    /// driver name contains it (drivers append version noise).
    pub fn lookup(&self, driver: &str) -> PrinterQuirks {
        let d = driver.to_ascii_lowercase();
        self.by_driver
            .iter()
            .find(|(k, _)| d.contains(&k.to_ascii_lowercase()))
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    }
    pub fn merge_overrides(&mut self, overrides: ProfileDb) {
        for (k, v) in overrides.by_driver {
            self.by_driver.insert(k, v);
        }
        self.version = self.version.max(overrides.version);
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }
}

impl Default for ProfileDb {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lookup_and_override() {
        let mut db = ProfileDb::new();
        db.by_driver.insert(
            "EPSON TM-T88".into(),
            PrinterQuirks {
                offset_x_mm: -1.5,
                force_threshold: true,
                ..Default::default()
            },
        );
        let q = db.lookup("EPSON TM-T88VII Series");
        assert_eq!(q.offset_x_mm, -1.5);
        assert!(q.force_threshold);
        assert_eq!(db.lookup("Unknown Driver").offset_x_mm, 0.0);
        let mut ov = ProfileDb::new();
        ov.by_driver.insert(
            "EPSON TM-T88".into(),
            PrinterQuirks {
                offset_x_mm: -2.0,
                ..Default::default()
            },
        );
        db.merge_overrides(ov);
        assert_eq!(db.lookup("epson tm-t88").offset_x_mm, -2.0);
        // Round-trip through JSON (the on-disk format).
        let db2 = ProfileDb::from_json(&db.to_json()).unwrap();
        assert_eq!(db2.lookup("EPSON TM-T88").offset_x_mm, -2.0);
    }
}
