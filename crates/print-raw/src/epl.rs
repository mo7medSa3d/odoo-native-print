use crate::mono::MonoBitmap;

/// EPL2 (legacy Zebra/Eltron) graphic write: GW x,y,bytesPerRow,height,<binary>
/// inside an N ... P1 label form. Kept for older label printers still in
/// the field; new deployments should prefer ZPL or TSPL.
pub fn epl_label(bm: &MonoBitmap) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"N\r\n");
    out.extend_from_slice(
        format!("GW{},{},{},{},", 0, 0, bm.bytes_per_row(), bm.height_dots).as_bytes(),
    );
    out.extend_from_slice(&bm.data);
    out.extend_from_slice(b"\r\nP1\r\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn epl_shape() {
        let bm = MonoBitmap::from_packed(8, 1, vec![0xAA]).unwrap();
        let out = epl_label(&bm);
        assert!(out.starts_with(b"N\r\nGW0,0,1,1,"));
        assert!(out.ends_with(b"\r\nP1\r\n"));
        assert!(out.windows(1).any(|w| w == [0xAA]));
    }
}
