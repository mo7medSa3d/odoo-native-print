use crate::mono::MonoBitmap;

/// ZPL ^GFA graphic field: ^GFA,<total_bytes>,<total_bytes>,<bytes_per_row>,<hex data>
/// Hex is uppercase ASCII, one nibble pair per byte, no compression (correct
/// first; compression is an optimisation, not a correctness feature).
pub fn zpl_graphic_field(bm: &MonoBitmap) -> String {
    let total = bm.data.len();
    let bpr = bm.bytes_per_row();
    let mut hex = String::with_capacity(total * 2);
    for b in &bm.data {
        hex.push_str(&format!("{b:02X}"));
    }
    format!("^GFA,{total},{total},{bpr},{hex}")
}

pub fn zpl_label(bm: &MonoBitmap) -> String {
    format!("^XA{}^FO0,0^XZ", zpl_graphic_field(bm))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gfa_header_and_hex() {
        let bm = MonoBitmap::from_packed(8, 1, vec![0b1010_1010]).unwrap();
        assert_eq!(zpl_graphic_field(&bm), "^GFA,1,1,1,AA");
        let label = zpl_label(&bm);
        assert!(label.starts_with("^XA") && label.ends_with("^XZ"));
    }
}
