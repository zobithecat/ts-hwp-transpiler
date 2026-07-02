//! ParagraphHeader (tag 0x0050) record codec.
//!
//! Layout (hwplib `ParagraphHeaderReader`):
//!
//!   u32  char_count           // UTF-16 code units
//!   u32  control_mask
//!   u16  para_shape_id
//!   u8   style_id
//!   i8   divide_sort
//!   u16  char_shape_count
//!   u16  range_tag_count
//!   u16  line_align_count
//!   u32  instance_id
//!   [opt HWP 5.0.3.2+]  u16 change_tracking_merged
//!
//! Base size 22; with merge flag 24.

use hwp_transpiler_core::ir::{IrError, ParagraphHeader, Record};

use super::body_text::tag;

pub const BASE_SIZE: usize = 4 + 4 + 2 + 1 + 1 + 2 + 2 + 2 + 4; // 22
pub const WITH_MERGE: usize = BASE_SIZE + 2; // 24

pub fn parse(rec: &Record) -> Result<ParagraphHeader, IrError> {
    if rec.tag != tag::PARA_HEADER {
        return Err(IrError::Invalid(format!(
            "expected PARA_HEADER (0x{:04X}), got 0x{:04X}",
            tag::PARA_HEADER,
            rec.tag
        )));
    }
    if rec.data.len() < BASE_SIZE {
        return Err(IrError::Invalid(format!(
            "ParagraphHeader too short: {} < {BASE_SIZE}",
            rec.data.len()
        )));
    }
    let mut c = Cur::new(&rec.data);
    let char_count = c.u32()?;
    let control_mask = c.u32()?;
    let para_shape_id = c.u16()?;
    let style_id = c.u8()?;
    let divide_sort = c.i8()?;
    let char_shape_count = c.u16()?;
    let range_tag_count = c.u16()?;
    let line_align_count = c.u16()?;
    let instance_id = c.u32()?;
    let change_tracking_merged = (rec.data.len() >= WITH_MERGE).then(|| c.u16()).transpose()?;

    Ok(ParagraphHeader {
        char_count,
        control_mask,
        para_shape_id,
        style_id,
        divide_sort,
        // HWP5 divide kind bit 2 (0x04) = start on a new page. Kept
        // in sync with `divide_sort` so HWPX emission of an HWP5
        // source reproduces forced page breaks.
        page_break_before: divide_sort & 0x04 != 0,
        char_shape_count,
        range_tag_count,
        line_align_count,
        instance_id,
        change_tracking_merged,
    })
}

pub fn emit(h: &ParagraphHeader) -> Vec<u8> {
    let mut out = Vec::with_capacity(WITH_MERGE);
    out.extend_from_slice(&h.char_count.to_le_bytes());
    out.extend_from_slice(&h.control_mask.to_le_bytes());
    out.extend_from_slice(&h.para_shape_id.to_le_bytes());
    out.push(h.style_id);
    out.push(h.divide_sort as u8);
    out.extend_from_slice(&h.char_shape_count.to_le_bytes());
    out.extend_from_slice(&h.range_tag_count.to_le_bytes());
    out.extend_from_slice(&h.line_align_count.to_le_bytes());
    out.extend_from_slice(&h.instance_id.to_le_bytes());
    if let Some(m) = h.change_tracking_merged {
        out.extend_from_slice(&m.to_le_bytes());
    }
    out
}

struct Cur<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cur<'a> {
    fn new(bytes: &'a [u8]) -> Self { Self { bytes, pos: 0 } }
    fn chunk<const N: usize>(&mut self) -> Result<[u8; N], IrError> {
        if self.pos + N > self.bytes.len() {
            return Err(IrError::Invalid(format!(
                "ParagraphHeader: {N}-byte OOB at {}",
                self.pos
            )));
        }
        let mut b = [0u8; N];
        b.copy_from_slice(&self.bytes[self.pos..self.pos + N]);
        self.pos += N;
        Ok(b)
    }
    fn u8(&mut self) -> Result<u8, IrError> { Ok(self.chunk::<1>()?[0]) }
    fn i8(&mut self) -> Result<i8, IrError> { Ok(self.chunk::<1>()?[0] as i8) }
    fn u16(&mut self) -> Result<u16, IrError> { Ok(u16::from_le_bytes(self.chunk::<2>()?)) }
    fn u32(&mut self) -> Result<u32, IrError> { Ok(u32::from_le_bytes(self.chunk::<4>()?)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(data: Vec<u8>) -> Record {
        Record { tag: tag::PARA_HEADER, level: 0, data }
    }

    fn sample() -> ParagraphHeader {
        ParagraphHeader {
            char_count: 42,
            control_mask: 0,
            para_shape_id: 1,
            style_id: 0,
            divide_sort: 0,
            char_shape_count: 1,
            range_tag_count: 0,
            line_align_count: 1,
            instance_id: 0,
            change_tracking_merged: None,
            page_break_before: false,
        }
    }

    #[test]
    fn base_roundtrip() {
        let h = sample();
        let bytes = emit(&h);
        assert_eq!(bytes.len(), BASE_SIZE);
        assert_eq!(parse(&rec(bytes)).unwrap(), h);
    }

    #[test]
    fn with_merge_flag_roundtrip() {
        let mut h = sample();
        h.change_tracking_merged = Some(0);
        let bytes = emit(&h);
        assert_eq!(bytes.len(), WITH_MERGE);
        assert_eq!(parse(&rec(bytes)).unwrap(), h);
    }

    #[test]
    fn wrong_tag_errors() {
        let r = Record { tag: tag::PARA_TEXT, level: 0, data: vec![0; BASE_SIZE] };
        assert!(parse(&r).is_err());
    }

    #[test]
    fn short_errors() {
        let r = rec(vec![0; BASE_SIZE - 1]);
        assert!(parse(&r).is_err());
    }
}
