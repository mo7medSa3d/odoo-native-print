use thiserror::Error;

#[derive(Debug, Error)]
pub enum MonoError {
    #[error("bitmap width {0} is not a multiple of 8 and cannot be packed without an explicit pad policy")]
    WidthNotMultipleOf8(u32),
    #[error("bitmap data length mismatch: expected {expected}, got {got}")]
    LengthMismatch { expected: usize, got: usize },
    #[error("1-bit threshold input must be grayscale bytes (1 byte per pixel)")]
    BadInput,
}

/// A 1-bit monochrome bitmap, packed MSB-first, row-major.
/// This is the internal thermal page model: no dithering lives here —
/// thresholding happens when a grayscale page is converted, with a
/// caller-chosen threshold (thermal/barcode work uses a hard threshold).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonoBitmap {
    pub width_dots: u32,
    pub height_dots: u32,
    /// Packed bits, len == (width/8) * height. 1 = black dot.
    pub data: Vec<u8>,
}

impl MonoBitmap {
    pub fn bytes_per_row(&self) -> usize {
        (self.width_dots as usize).div_ceil(8)
    }

    /// Threshold a grayscale image (1 byte/pixel, 0=black..255=white).
    /// Pixels darker than `threshold` become black dots. Rows are padded
    /// with white to a whole byte, which is the safe thermal behaviour
    /// (padding never prints).
    pub fn from_grayscale(
        gray: &[u8],
        width: u32,
        height: u32,
        threshold: u8,
    ) -> Result<Self, MonoError> {
        let expected = (width as usize)
            .checked_mul(height as usize)
            .ok_or(MonoError::BadInput)?;
        if gray.len() != expected {
            return Err(MonoError::LengthMismatch {
                expected,
                got: gray.len(),
            });
        }
        let bpr = (width as usize).div_ceil(8);
        let mut data = vec![0u8; bpr * height as usize];
        for y in 0..height as usize {
            for x in 0..width as usize {
                if gray[y * width as usize + x] < threshold {
                    data[y * bpr + x / 8] |= 0x80 >> (x % 8);
                }
            }
        }
        Ok(Self {
            width_dots: width,
            height_dots: height,
            data,
        })
    }

    /// Strict constructor for already-packed data (width must fit exactly).
    pub fn from_packed(
        width_dots: u32,
        height_dots: u32,
        data: Vec<u8>,
    ) -> Result<Self, MonoError> {
        if width_dots % 8 != 0 {
            return Err(MonoError::WidthNotMultipleOf8(width_dots));
        }
        let expected = (width_dots as usize / 8) * height_dots as usize;
        if data.len() != expected {
            return Err(MonoError::LengthMismatch {
                expected,
                got: data.len(),
            });
        }
        Ok(Self {
            width_dots,
            height_dots,
            data,
        })
    }

    pub fn pixel_is_black(&self, x: u32, y: u32) -> bool {
        let idx = y as usize * self.bytes_per_row() + (x as usize / 8);
        self.data[idx] & (0x80 >> (x % 8)) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn threshold_and_packing() {
        // 8x1: black, white, black... threshold 128
        let gray = [0u8, 255, 0, 255, 0, 255, 0, 255];
        let bm = MonoBitmap::from_grayscale(&gray, 8, 1, 128).unwrap();
        assert_eq!(bm.data, vec![0b1010_1010]);
        assert!(bm.pixel_is_black(0, 0));
        assert!(!bm.pixel_is_black(1, 0));
    }
    #[test]
    fn width_padding_is_white() {
        // 10 wide, all black -> 2 bytes/row: 11111111 11000000
        let gray = vec![0u8; 10];
        let bm = MonoBitmap::from_grayscale(&gray, 10, 1, 128).unwrap();
        assert_eq!(bm.data, vec![0xFF, 0b1100_0000]);
    }
    #[test]
    fn bad_lengths_rejected() {
        assert!(MonoBitmap::from_grayscale(&[0u8; 3], 8, 1, 128).is_err());
        assert!(MonoBitmap::from_packed(7, 1, vec![0]).is_err());
    }
}
