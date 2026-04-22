//! LIST_HEADER (tag 0x0048). Appears in multiple contexts:
//!
//! - Section top-level (before the first paragraph of the section).
//! - Each table cell, following the TABLE record.
//! - Footnote / endnote / header / footer containers.
//!
//! Cell layout (from hwplib `reader/.../tbl/ForCell.java::listHeader`):
//!
//!   sInt4  paraCount
//!   uInt4  property
//!   uInt2  colIndex      uInt2  rowIndex
//!   uInt2  colSpan       uInt2  rowSpan
//!   uInt4  width         uInt4  height
//!   uInt2  leftMargin    uInt2  rightMargin
//!   uInt2  topMargin     uInt2  bottomMargin
//!   uInt2  borderFillId
//!   uInt4  textWidth                             ← (38 bytes fixed)
//!   [opt]  uInt1 fieldNameFlag (0xff → ParameterSet) + 8-byte zero pad
//!
//! We parse offset-from-start (the trailing optional bytes make the prior
//! offset-from-end strategy unsafe).

use hwp_transpiler_core::ir::{IrError, Record, TableCell};

use super::body_text::tag;

const CELL_FIXED: usize = 38;

pub fn parse_cell(rec: &Record) -> Result<TableCell, IrError> {
    if rec.tag != tag::LIST_HEADER {
        return Err(IrError::Invalid(format!(
            "expected LIST_HEADER (0x{:04X}), got 0x{:04X}",
            tag::LIST_HEADER,
            rec.tag
        )));
    }
    if rec.data.len() < CELL_FIXED {
        return Err(IrError::Invalid(format!(
            "LIST_HEADER (cell) too short: {} < {CELL_FIXED}",
            rec.data.len()
        )));
    }
    let b = &rec.data;
    let u16_at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
    let u32_at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let i32_at = |o: usize| i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);

    Ok(TableCell {
        para_count: i32_at(0),
        property: u32_at(4),
        col: u16_at(8),
        row: u16_at(10),
        col_span: u16_at(12),
        row_span: u16_at(14),
        width_hwpu: u32_at(16),
        height_hwpu: u32_at(20),
        padding_hwpu: [u16_at(24), u16_at(26), u16_at(28), u16_at(30)],
        border_fill_id: u16_at(32),
        text_width_hwpu: u32_at(34),
        paragraphs: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(data: Vec<u8>) -> Record {
        Record { tag: tag::LIST_HEADER, level: 2, data }
    }

    /// Build a 38-byte cell LIST_HEADER matching hwplib's serialized layout.
    fn build(col: u16, row: u16, cs: u16, rs: u16, bf: u16) -> Vec<u8> {
        let mut out = Vec::with_capacity(CELL_FIXED);
        out.extend_from_slice(&1i32.to_le_bytes());     // paraCount
        out.extend_from_slice(&0u32.to_le_bytes());     // property
        out.extend_from_slice(&col.to_le_bytes());
        out.extend_from_slice(&row.to_le_bytes());
        out.extend_from_slice(&cs.to_le_bytes());
        out.extend_from_slice(&rs.to_le_bytes());
        out.extend_from_slice(&1000u32.to_le_bytes()); // width
        out.extend_from_slice(&500u32.to_le_bytes());  // height
        for p in [10u16, 20, 30, 40] {                 // margins L/R/T/B
            out.extend_from_slice(&p.to_le_bytes());
        }
        out.extend_from_slice(&bf.to_le_bytes());      // borderFillId
        out.extend_from_slice(&900u32.to_le_bytes()); // textWidth
        debug_assert_eq!(out.len(), CELL_FIXED);
        out
    }

    #[test]
    fn parses_38_byte_cell() {
        let data = build(0, 0, 1, 1, 7);
        let cell = parse_cell(&rec(data)).unwrap();
        assert_eq!(cell.para_count, 1);
        assert_eq!(cell.col, 0);
        assert_eq!(cell.row, 0);
        assert_eq!(cell.col_span, 1);
        assert_eq!(cell.row_span, 1);
        assert_eq!(cell.width_hwpu, 1000);
        assert_eq!(cell.height_hwpu, 500);
        assert_eq!(cell.padding_hwpu, [10, 20, 30, 40]);
        assert_eq!(cell.border_fill_id, 7);
        assert_eq!(cell.text_width_hwpu, 900);
    }

    #[test]
    fn parses_merged_cell() {
        let data = build(2, 3, 4, 2, 1);
        let cell = parse_cell(&rec(data)).unwrap();
        assert_eq!(cell.col, 2);
        assert_eq!(cell.row, 3);
        assert_eq!(cell.col_span, 4);
        assert_eq!(cell.row_span, 2);
    }

    /// Hwplib's writer always appends `uInt1 flag + 8-byte zero pad` after
    /// the 38-byte fixed block (47 bytes total when there's no field name).
    /// Earlier offset-from-end logic mistakenly read into this trailer and
    /// produced garbage like `col=65024, col_span=36097`. Lock in the fix.
    #[test]
    fn parses_47_byte_cell_with_trailer() {
        let mut data = build(0, 11, 1, 1, 3);
        data.push(0x00);                  // flag (no fieldName)
        data.extend_from_slice(&[0u8; 8]); // zero pad
        assert_eq!(data.len(), 47);
        let cell = parse_cell(&rec(data)).unwrap();
        assert_eq!(cell.col, 0);
        assert_eq!(cell.row, 11);
        assert_eq!(cell.col_span, 1);
        assert_eq!(cell.row_span, 1);
        assert_eq!(cell.border_fill_id, 3);
    }

    #[test]
    fn wrong_tag_errors() {
        let r = Record { tag: tag::TABLE, level: 0, data: vec![0; CELL_FIXED] };
        assert!(parse_cell(&r).is_err());
    }

    #[test]
    fn too_short_errors() {
        let r = rec(vec![0; CELL_FIXED - 1]);
        assert!(parse_cell(&r).is_err());
    }
}
