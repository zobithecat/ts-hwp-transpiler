//! TABLE (tag 0x004D) — appears immediately after a CTRL_HEADER with the
//! "tbl " id at level ctrl+1. Carries the grid geometry; individual cell
//! records (LIST_HEADER 0x0048 + cell paragraphs) follow.
//!
//! Layout:
//!
//!   u32  attribute
//!   u16  rows
//!   u16  cols
//!   i16  cell_spacing
//!   i16×4 padding                       // [left, right, top, bottom]
//!   u16×rows row_cell_counts            // cells per row (uneven rows OK)
//!   u16  border_fill_id
//!   [opt HWP 5.0.1.0+]
//!     u16 valid_zone_count
//!     ValidZone × valid_zone_count      // each = 5 × u16 = 10 bytes

use hwp_transpiler_core::ir::{IrError, Record, TableControl, ValidZone};

use super::body_text::tag;

const FIXED_PREFIX: usize = 4 + 2 + 2 + 2 + 2 * 4; // 18 bytes

pub fn parse(rec: &Record) -> Result<TableControl, IrError> {
    if rec.tag != tag::TABLE {
        return Err(IrError::Invalid(format!(
            "expected TABLE (0x{:04X}), got 0x{:04X}",
            tag::TABLE,
            rec.tag
        )));
    }
    if rec.data.len() < FIXED_PREFIX {
        return Err(IrError::Invalid(format!(
            "TABLE too short: {} < {FIXED_PREFIX}",
            rec.data.len()
        )));
    }

    let mut c = Cur::new(&rec.data);
    let attribute = c.u32()?;
    let rows = c.u16()?;
    let cols = c.u16()?;
    let cell_spacing = c.i16()?;
    let padding = [c.i16()?, c.i16()?, c.i16()?, c.i16()?];

    let mut row_cell_counts = Vec::with_capacity(rows as usize);
    for _ in 0..rows {
        row_cell_counts.push(c.u16()?);
    }
    let border_fill_id = c.u16()?;

    let mut valid_zones = Vec::new();
    if c.remaining() >= 2 {
        let vz_count = c.u16()?;
        for _ in 0..vz_count {
            valid_zones.push(ValidZone {
                start_col: c.u16()?,
                start_row: c.u16()?,
                end_col: c.u16()?,
                end_row: c.u16()?,
                border_fill_id: c.u16()?,
            });
        }
    }

    Ok(TableControl {
        attribute,
        rows,
        cols,
        cell_spacing,
        padding,
        row_cell_counts,
        border_fill_id,
        valid_zones,
        cells: Vec::new(),
    })
}

pub fn emit(t: &TableControl) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&t.attribute.to_le_bytes());
    out.extend_from_slice(&t.rows.to_le_bytes());
    out.extend_from_slice(&t.cols.to_le_bytes());
    out.extend_from_slice(&t.cell_spacing.to_le_bytes());
    for p in t.padding {
        out.extend_from_slice(&p.to_le_bytes());
    }
    for r in &t.row_cell_counts {
        out.extend_from_slice(&r.to_le_bytes());
    }
    out.extend_from_slice(&t.border_fill_id.to_le_bytes());
    out.extend_from_slice(&(t.valid_zones.len() as u16).to_le_bytes());
    for z in &t.valid_zones {
        out.extend_from_slice(&z.start_col.to_le_bytes());
        out.extend_from_slice(&z.start_row.to_le_bytes());
        out.extend_from_slice(&z.end_col.to_le_bytes());
        out.extend_from_slice(&z.end_row.to_le_bytes());
        out.extend_from_slice(&z.border_fill_id.to_le_bytes());
    }
    out
}

struct Cur<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cur<'a> {
    fn new(bytes: &'a [u8]) -> Self { Self { bytes, pos: 0 } }
    fn remaining(&self) -> usize { self.bytes.len() - self.pos }
    fn chunk<const N: usize>(&mut self) -> Result<[u8; N], IrError> {
        if self.pos + N > self.bytes.len() {
            return Err(IrError::Invalid(format!(
                "TABLE: {N}-byte OOB at {}",
                self.pos
            )));
        }
        let mut b = [0u8; N];
        b.copy_from_slice(&self.bytes[self.pos..self.pos + N]);
        self.pos += N;
        Ok(b)
    }
    fn u16(&mut self) -> Result<u16, IrError> { Ok(u16::from_le_bytes(self.chunk::<2>()?)) }
    fn i16(&mut self) -> Result<i16, IrError> { Ok(i16::from_le_bytes(self.chunk::<2>()?)) }
    fn u32(&mut self) -> Result<u32, IrError> { Ok(u32::from_le_bytes(self.chunk::<4>()?)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(data: Vec<u8>) -> Record {
        Record { tag: tag::TABLE, level: 2, data }
    }

    fn table_2x2() -> TableControl {
        TableControl {
            attribute: 0,
            rows: 2,
            cols: 2,
            cell_spacing: 0,
            padding: [0; 4],
            row_cell_counts: vec![2, 2],
            border_fill_id: 1,
            valid_zones: Vec::new(),
            cells: Vec::new(),
        }
    }

    #[test]
    fn roundtrip_2x2() {
        let t = table_2x2();
        let bytes = emit(&t);
        assert_eq!(parse(&rec(bytes)).unwrap(), t);
    }

    #[test]
    fn roundtrip_with_valid_zones() {
        let mut t = table_2x2();
        t.valid_zones = vec![
            ValidZone { start_col: 0, start_row: 0, end_col: 1, end_row: 0, border_fill_id: 2 },
            ValidZone { start_col: 0, start_row: 1, end_col: 1, end_row: 1, border_fill_id: 3 },
        ];
        let bytes = emit(&t);
        assert_eq!(parse(&rec(bytes)).unwrap(), t);
    }

    #[test]
    fn ragged_rows() {
        let mut t = table_2x2();
        t.rows = 3;
        t.cols = 4;
        t.row_cell_counts = vec![4, 2, 3]; // merged cells in row 1/2
        let bytes = emit(&t);
        assert_eq!(parse(&rec(bytes)).unwrap(), t);
    }

    #[test]
    fn wrong_tag_errors() {
        let r = Record { tag: tag::LIST_HEADER, level: 0, data: vec![0; FIXED_PREFIX] };
        assert!(parse(&r).is_err());
    }

    #[test]
    fn too_short_errors() {
        assert!(parse(&rec(vec![0; FIXED_PREFIX - 1])).is_err());
    }
}
