use serde::{Deserialize, Serialize};

/// A page size in millimetres. Millimetres are the single internal unit in
/// print-core; conversions to PDF points (1/72") and printer device units
/// happen at the platform boundary, in one tested place.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PageSize {
    pub width_mm: f64,
    pub height_mm: f64,
}

impl PageSize {
    pub fn new(width_mm: f64, height_mm: f64) -> Self {
        Self {
            width_mm,
            height_mm,
        }
    }
    pub fn a4() -> Self {
        Self::new(210.0, 297.0)
    }
    pub fn a5() -> Self {
        Self::new(148.0, 210.0)
    }
    /// 80 mm thermal roll: width fixed, height is content dependent.
    pub fn roll_80(height_mm: f64) -> Self {
        Self::new(80.0, height_mm)
    }
    pub fn label_100x150() -> Self {
        Self::new(100.0, 150.0)
    }

    pub fn width_points(&self) -> f64 {
        self.width_mm * 72.0 / 25.4
    }
    pub fn height_points(&self) -> f64 {
        self.height_mm * 72.0 / 25.4
    }
    pub fn rotated(&self) -> Self {
        Self::new(self.height_mm, self.width_mm)
    }
    pub fn is_landscape(&self) -> bool {
        self.width_mm > self.height_mm
    }
    /// Pixel dimensions when rendered at `dpi`, rounded (never truncated,
    /// so a page never loses its last row/column of dots).
    pub fn pixels_at(&self, dpi: u32) -> (u32, u32) {
        let to_px = |mm: f64| ((mm / 25.4) * dpi as f64).round().max(1.0) as u32;
        (to_px(self.width_mm), to_px(self.height_mm))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rotation {
    R0,
    R90,
    R180,
    R270,
}

impl Rotation {
    pub fn from_pdf_degrees(deg: i32) -> Self {
        match deg.rem_euclid(360) {
            90 => Rotation::R90,
            180 => Rotation::R180,
            270 => Rotation::R270,
            _ => Rotation::R0,
        }
    }
    /// Effective size after applying this rotation to a page.
    pub fn apply(&self, size: PageSize) -> PageSize {
        match self {
            Rotation::R90 | Rotation::R270 => size.rotated(),
            _ => size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a4_points() {
        let a4 = PageSize::a4();
        assert!((a4.width_points() - 595.28).abs() < 0.5);
        assert!((a4.height_points() - 841.89).abs() < 0.5);
    }
    #[test]
    fn a4_pixels_300dpi() {
        assert_eq!(PageSize::a4().pixels_at(300), (2480, 3508));
    }
    #[test]
    fn rotation_swaps_size() {
        assert_eq!(
            Rotation::R90.apply(PageSize::a4()),
            PageSize::a4().rotated()
        );
        assert_eq!(Rotation::from_pdf_degrees(-90), Rotation::R270);
    }
}
