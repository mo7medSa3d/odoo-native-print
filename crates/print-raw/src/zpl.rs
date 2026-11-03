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

/// Complete ZPL II label format: `^FO0,0` (Field Origin) must precede `^GFA`
/// and end with `^FS` (Field Separator) before `^XZ`.
pub fn zpl_label(bm: &MonoBitmap) -> String {
    zpl_label_with_options(bm, 0, 0, 1)
}

/// Complete ZPL II label with print width (`^PW`), label length (`^LL`),
/// label home offset (`^LH`), field origin (`^FO0,0`), graphic field (`^GFA`),
/// field separator (`^FS`), and print quantity (`^PQ`).
pub fn zpl_label_with_options(
    bm: &MonoBitmap,
    offset_x_dots: i32,
    offset_y_dots: i32,
    copies: u16,
) -> String {
    let copies = copies.max(1);
    let lh_x = offset_x_dots.max(0);
    let lh_y = offset_y_dots.max(0);
    format!(
        "^XA^PW{}^LL{}^LH{lh_x},{lh_y}^FO0,0{}^FS^PQ{copies},0,1,Y^XZ",
        bm.width_dots.max(2),
        bm.height_dots.max(1),
        zpl_graphic_field(bm)
    )
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
        assert!(label.contains("^FO0,0^GFA,1,1,1,AA^FS"));
    }

    #[test]
    fn label_options_include_dimensions_and_copies() {
        let bm = MonoBitmap::from_packed(16, 4, vec![0xFF; 8]).unwrap();
        let label = zpl_label_with_options(&bm, 12, 8, 3);
        assert!(label.contains("^PW16^LL4^LH12,8^FO0,0^GFA,8,8,2,"));
        assert!(label.ends_with("^FS^PQ3,0,1,Y^XZ"));
    }
}
