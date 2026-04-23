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

/// Inverse of `parse_cell`. Emits the 47-byte form hwplib's writer
/// always produces: 38-byte fixed block + trailing `uInt1 flag + 8-byte
/// zero pad`. `parse_cell` accepts both the 38- and 47-byte forms, so
/// round-tripping through emit → parse is lossless. `cell.para_count`
/// is auto-synced from `cell.paragraphs.len()` so callers that insert
/// or remove paragraphs don't have to remember to bump the stored
/// counter.
pub fn emit_cell(cell: &TableCell) -> Vec<u8> {
    const FULL: usize = 47;
    let mut out = Vec::with_capacity(FULL);
    let para_count = cell.paragraphs.len() as i32;
    out.extend_from_slice(&para_count.to_le_bytes());
    out.extend_from_slice(&cell.property.to_le_bytes());
    out.extend_from_slice(&cell.col.to_le_bytes());
    out.extend_from_slice(&cell.row.to_le_bytes());
    out.extend_from_slice(&cell.col_span.to_le_bytes());
    out.extend_from_slice(&cell.row_span.to_le_bytes());
    out.extend_from_slice(&cell.width_hwpu.to_le_bytes());
    out.extend_from_slice(&cell.height_hwpu.to_le_bytes());
    for p in cell.padding_hwpu {
        out.extend_from_slice(&p.to_le_bytes());
    }
    out.extend_from_slice(&cell.border_fill_id.to_le_bytes());
    out.extend_from_slice(&cell.text_width_hwpu.to_le_bytes());
    // Trailer: fieldName flag = 0 (no ParameterSet), then 8-byte pad.
    out.push(0x00);
    out.extend_from_slice(&[0u8; 8]);
    debug_assert_eq!(out.len(), FULL);
    out
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

    fn sample_cell() -> TableCell {
        use hwp_transpiler_core::ir::{Paragraph, TableCell};
        TableCell {
            para_count: 1,
            property: 0x12345678,
            col: 3,
            row: 2,
            col_span: 2,
            row_span: 4,
            width_hwpu: 5000,
            height_hwpu: 2500,
            padding_hwpu: [11, 22, 33, 44],
            border_fill_id: 9,
            text_width_hwpu: 4900,
            paragraphs: vec![Paragraph::default()],
        }
    }

    #[test]
    fn emit_cell_produces_47_byte_form() {
        let bytes = emit_cell(&sample_cell());
        assert_eq!(bytes.len(), 47);
    }

    #[test]
    fn emit_parse_cell_roundtrips() {
        let c = sample_cell();
        let bytes = emit_cell(&c);
        let parsed = parse_cell(&rec(bytes)).unwrap();
        assert_eq!(parsed.property, c.property);
        assert_eq!(parsed.col, c.col);
        assert_eq!(parsed.row, c.row);
        assert_eq!(parsed.col_span, c.col_span);
        assert_eq!(parsed.row_span, c.row_span);
        assert_eq!(parsed.width_hwpu, c.width_hwpu);
        assert_eq!(parsed.height_hwpu, c.height_hwpu);
        assert_eq!(parsed.padding_hwpu, c.padding_hwpu);
        assert_eq!(parsed.border_fill_id, c.border_fill_id);
        assert_eq!(parsed.text_width_hwpu, c.text_width_hwpu);
        // para_count comes from paragraphs.len() on emit — parsed
        // value mirrors that, not the stored `c.para_count`.
        assert_eq!(parsed.para_count, c.paragraphs.len() as i32);
    }

    #[test]
    fn emit_cell_para_count_tracks_paragraphs_len() {
        use hwp_transpiler_core::ir::Paragraph;
        let mut c = sample_cell();
        c.para_count = 99; // stale value — emit should ignore it.
        c.paragraphs = vec![Paragraph::default(), Paragraph::default(), Paragraph::default()];
        let parsed = parse_cell(&rec(emit_cell(&c))).unwrap();
        assert_eq!(parsed.para_count, 3, "emit must re-derive para_count");
    }
}
