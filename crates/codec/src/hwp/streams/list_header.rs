//! LIST_HEADER (tag 0x0048). Appears in multiple contexts:
//!
//! - Section top-level (before the first paragraph of the section).
//! - Each table cell, following the TABLE record.
//! - Footnote / endnote / header / footer containers.
//!
//! The record starts with a version-dependent "ParagraphList common" block
//! (paragraph_count + property + optional extras), followed by
//! container-specific fields. For the cell case, the container part is the
//! last 26 bytes:
//!
//!   u16  col              u16  row
//!   u16  col_span         u16  row_span
//!   u32  width_hwpu       u32  height_hwpu
//!   u16  left  right  top  bottom  (padding, HWPUNIT)
//!   u16  border_fill_id
//!
//! We parse the cell suffix by offset-from-end, treating everything before
//! it as opaque list-header preamble.

use hwp_transpiler_core::ir::{IrError, Record, TableCell};

use super::body_text::tag;

const CELL_SUFFIX: usize = 2 + 2 + 2 + 2 + 4 + 4 + 2 * 4 + 2; // 26

pub fn parse_cell(rec: &Record) -> Result<TableCell, IrError> {
    if rec.tag != tag::LIST_HEADER {
        return Err(IrError::Invalid(format!(
            "expected LIST_HEADER (0x{:04X}), got 0x{:04X}",
            tag::LIST_HEADER,
            rec.tag
        )));
    }
    if rec.data.len() < CELL_SUFFIX {
        return Err(IrError::Invalid(format!(
            "LIST_HEADER (cell) too short: {} < {CELL_SUFFIX}",
            rec.data.len()
        )));
    }
    let off = rec.data.len() - CELL_SUFFIX;
    let b = &rec.data[off..];
    let u16_at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
    let u32_at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);

    Ok(TableCell {
        col: u16_at(0),
        row: u16_at(2),
        col_span: u16_at(4),
        row_span: u16_at(6),
        width_hwpu: u32_at(8),
        height_hwpu: u32_at(12),
        padding_hwpu: [u16_at(16), u16_at(18), u16_at(20), u16_at(22)],
        border_fill_id: u16_at(24),
        paragraphs: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(data: Vec<u8>) -> Record {
        Record { tag: tag::LIST_HEADER, level: 2, data }
    }

    /// Build a plausible 47-byte cell LIST_HEADER: 21-byte common prefix
    /// (content irrelevant) + 26-byte cell suffix.
    fn build(col: u16, row: u16, cs: u16, rs: u16, bf: u16) -> Vec<u8> {
        let mut out = vec![0u8; 21];
        out.extend_from_slice(&col.to_le_bytes());
        out.extend_from_slice(&row.to_le_bytes());
        out.extend_from_slice(&cs.to_le_bytes());
        out.extend_from_slice(&rs.to_le_bytes());
        out.extend_from_slice(&1000u32.to_le_bytes()); // width
        out.extend_from_slice(&500u32.to_le_bytes()); // height
        for p in [10u16, 20, 30, 40] {
            out.extend_from_slice(&p.to_le_bytes());
        }
        out.extend_from_slice(&bf.to_le_bytes());
        out
    }

    #[test]
    fn parses_47_byte_cell() {
        let data = build(0, 0, 1, 1, 7);
        assert_eq!(data.len(), 47);
        let cell = parse_cell(&rec(data)).unwrap();
        assert_eq!(cell.col, 0);
        assert_eq!(cell.col_span, 1);
        assert_eq!(cell.row_span, 1);
        assert_eq!(cell.width_hwpu, 1000);
        assert_eq!(cell.padding_hwpu, [10, 20, 30, 40]);
        assert_eq!(cell.border_fill_id, 7);
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

    #[test]
    fn wrong_tag_errors() {
        let r = Record { tag: tag::TABLE, level: 0, data: vec![0; 47] };
        assert!(parse_cell(&r).is_err());
    }

    #[test]
    fn too_short_errors() {
        let r = rec(vec![0; CELL_SUFFIX - 1]);
        assert!(parse_cell(&r).is_err());
    }
}
