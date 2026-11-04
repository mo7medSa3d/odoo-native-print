use crate::mono::MonoBitmap;

/// EPL2 (legacy Zebra/Eltron) graphic write: GW x,y,bytesPerRow,height,<binary>
/// inside an N ... P1 label form.
///
/// EPL2 GW specification (Zebra EPL2 Programmer's Manual):
/// In EPL2 binary graphic data, bit `0` = black dot (burn) and bit `1` = white
/// (unburned), the inverse of `MonoBitmap` (`1` = black). Each byte is therefore
/// bitwise-inverted (`!b`).
pub fn epl_label(bm: &MonoBitmap) -> Vec<u8> {
    epl_label_copies(bm, 1)
}

/// EPL2 graphic label with explicit copy count (`P<sets>`).
/// The gap parameter is the printer's 24-dot default (3 mm at
/// 203 dpi); use [`epl_label_copies_with_gap`] when the job
/// carries a configured gap.
pub fn epl_label_copies(bm: &MonoBitmap, copies: u16) -> Vec<u8> {
    epl_label_copies_with_gap(bm, copies, 24)
}

/// EPL2 graphic label with an explicit gap, in dots at the
/// render resolution (`Q<height>,<gap>`). The caller converts the
/// job's gap in millimetres at the render DPI; 24 dots is 3 mm
/// at 203 dpi.
pub fn epl_label_copies_with_gap(bm: &MonoBitmap, copies: u16, gap_dots: u32) -> Vec<u8> {
    let copies = copies.max(1);
    let mut out = Vec::new();
    out.extend_from_slice(b"N\r\n");
    out.extend_from_slice(
        format!(
            "q{}\r\nQ{},{gap_dots}\r\nGW0,0,{},{},",
            bm.width_dots.max(8),
            bm.height_dots.max(1),
            bm.bytes_per_row(),
            bm.height_dots
        )
        .as_bytes(),
    );
    for &b in &bm.data {
        out.push(!b);
    }
    out.extend_from_slice(format!("\r\nP{copies}\r\n").as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn epl_shape() {
        let bm = MonoBitmap::from_packed(8, 1, vec![0xAA]).unwrap();
        let out = epl_label(&bm);
        assert!(out.starts_with(b"N\r\nq8\r\nQ1,24\r\nGW0,0,1,1,"));
        assert!(out.ends_with(b"\r\nP1\r\n"));
        // 0xAA inverted is 0x55 (EPL2: 0 = black dot, 1 = white).
        assert!(out.windows(1).any(|w| w == [0x55]));
    }

    #[test]
    fn epl_gap_is_threaded_not_hardcoded() {
        let bm = MonoBitmap::from_packed(8, 1, vec![0xAA]).unwrap();
        let out = epl_label_copies_with_gap(&bm, 1, 36);
        assert!(out.starts_with(b"N\r\nq8\r\nQ1,36\r\nGW0,0,1,1,"));
        // The default entry point keeps the 24-dot gap.
        assert!(epl_label_copies(&bm, 1).starts_with(b"N\r\nq8\r\nQ1,24\r\n"));
    }
}
