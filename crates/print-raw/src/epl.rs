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
pub fn epl_label_copies(bm: &MonoBitmap, copies: u16) -> Vec<u8> {
    let copies = copies.max(1);
    let mut out = Vec::new();
    out.extend_from_slice(b"N\r\n");
    out.extend_from_slice(
        format!(
            "q{}\r\nQ{},24\r\nGW0,0,{},{},",
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
}
