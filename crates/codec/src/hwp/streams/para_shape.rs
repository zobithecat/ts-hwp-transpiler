//! ParaShape (tag 0x0019) record codec.
//!
//! Layout (hwplib `ParaShapeReader`):
//!
//!   u32   attribute (bitfield)
//!   i32   left_margin
//!   i32   right_margin
//!   i32   indent
//!   i32   top_space
//!   i32   bottom_space
//!   i32   line_space_legacy
//!   u16   tab_def_id
//!   u16   heading_id
//!   u16   border_fill_id
//!   i16   left_border_space
//!   i16   right_border_space
//!   i16   top_border_space
//!   i16   bottom_border_space
//!   [opt HWP 5.0.1.7+]  u32 attribute2
//!   [opt HWP 5.0.2.5+]  u32 attribute3
//!   [opt HWP 5.0.2.5+]  u32 line_spacing
//!
//! `line_spacing_kind` is NOT a separate binary field — it's packed
//! into `attribute` bits 0–1 (per HWP 5.0 spec table 44):
//! 0 = PERCENT (글자에 따라 %), 1 = FIXED (고정값), 2 = SPACING_ONLY
//! (여백만 지정). The IR exposes it as a separate `Option<u32>` so
//! emitters don't have to mask the attribute themselves.
//!
//! Base size: 42 bytes. Growth: +4 → 46 → 50 → 54 bytes.

use hwp_transpiler_core::ir::{IrError, ParaShape, Record};

use super::doc_info::tag;

const BASE: usize = 4 + 6 * 4 + 3 * 2 + 4 * 2; // 42
const WITH_ATTR2: usize = BASE + 4;            // 46
const WITH_ATTR3: usize = WITH_ATTR2 + 4;      // 50
const WITH_LINE_SPACING: usize = WITH_ATTR3 + 4; // 54

pub fn parse(rec: &Record) -> Result<ParaShape, IrError> {
    if rec.tag != tag::PARA_SHAPE {
        return Err(IrError::Invalid(format!(
            "expected ParaShape (0x{:04X}), got 0x{:04X}",
            tag::PARA_SHAPE,
            rec.tag
        )));
    }
    if rec.data.len() < BASE {
        return Err(IrError::Invalid(format!(
            "ParaShape too short: {} < {BASE}",
            rec.data.len()
        )));
    }

    let mut c = Cur::new(&rec.data);
    let attribute = c.u32()?;
    let left_margin = c.i32()?;
    let right_margin = c.i32()?;
    let indent = c.i32()?;
    let top_space = c.i32()?;
    let bottom_space = c.i32()?;
    let line_space_legacy = c.i32()?;
    let tab_def_id = c.u16()?;
    let heading_id = c.u16()?;
    let border_fill_id = c.u16()?;
    let left_border_space = c.i16()?;
    let right_border_space = c.i16()?;
    let top_border_space = c.i16()?;
    let bottom_border_space = c.i16()?;

    let n = rec.data.len();
    let attribute2 = (n >= WITH_ATTR2).then(|| c.u32()).transpose()?;
    let attribute3 = (n >= WITH_ATTR3).then(|| c.u32()).transpose()?;
    let line_spacing = (n >= WITH_LINE_SPACING).then(|| c.u32()).transpose()?;
    // Kind is packed into attribute bits 0-1 (HWP 5.0 spec table 44):
    //   0 = PERCENT (글자에 따라), 1 = FIXED (고정값),
    //   2 = SPACING_ONLY (여백만 지정).
    // Surface it as an `Option<u32>` aligned with `line_spacing` so
    // both are present together (5.0.2.5+) or both absent.
    let line_spacing_kind = line_spacing.map(|_| attribute & 0x3);

    Ok(ParaShape {
        attribute,
        left_margin,
        right_margin,
        indent,
        top_space,
        bottom_space,
        line_space_legacy,
        tab_def_id,
        heading_id,
        border_fill_id,
        left_border_space,
        right_border_space,
        top_border_space,
        bottom_border_space,
        attribute2,
        attribute3,
        line_spacing_kind,
        line_spacing,
    })
}

pub fn emit(ps: &ParaShape) -> Vec<u8> {
    let mut out = Vec::with_capacity(WITH_LINE_SPACING);
    out.extend_from_slice(&ps.attribute.to_le_bytes());
    out.extend_from_slice(&ps.left_margin.to_le_bytes());
    out.extend_from_slice(&ps.right_margin.to_le_bytes());
    out.extend_from_slice(&ps.indent.to_le_bytes());
    out.extend_from_slice(&ps.top_space.to_le_bytes());
    out.extend_from_slice(&ps.bottom_space.to_le_bytes());
    out.extend_from_slice(&ps.line_space_legacy.to_le_bytes());
    out.extend_from_slice(&ps.tab_def_id.to_le_bytes());
    out.extend_from_slice(&ps.heading_id.to_le_bytes());
    out.extend_from_slice(&ps.border_fill_id.to_le_bytes());
    out.extend_from_slice(&ps.left_border_space.to_le_bytes());
    out.extend_from_slice(&ps.right_border_space.to_le_bytes());
    out.extend_from_slice(&ps.top_border_space.to_le_bytes());
    out.extend_from_slice(&ps.bottom_border_space.to_le_bytes());
    if let Some(a) = ps.attribute2 {
        out.extend_from_slice(&a.to_le_bytes());
    }
    if let Some(a) = ps.attribute3 {
        out.extend_from_slice(&a.to_le_bytes());
    }
    // `line_spacing_kind` is packed into `attribute` bits 0–1; the
    // serialized form has only the value as a u32. Mask the kind
    // back into attribute on emit so a parse → emit round-trip
    // stays bit-equal even if the IR was constructed manually.
    if let Some(v) = ps.line_spacing {
        // Patch attribute's low 2 bits with kind if it diverged.
        if let Some(k) = ps.line_spacing_kind {
            let k = k & 0x3;
            let live = ps.attribute & 0x3;
            if live != k {
                let patched = (ps.attribute & !0x3) | k;
                out[0..4].copy_from_slice(&patched.to_le_bytes());
            }
        }
        out.extend_from_slice(&v.to_le_bytes());
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
                "ParaShape: {N}-byte OOB at {}/{}",
                self.pos,
                self.bytes.len()
            )));
        }
        let mut b = [0u8; N];
        b.copy_from_slice(&self.bytes[self.pos..self.pos + N]);
        self.pos += N;
        Ok(b)
    }

    fn u16(&mut self) -> Result<u16, IrError> {
        Ok(u16::from_le_bytes(self.chunk::<2>()?))
    }
    fn i16(&mut self) -> Result<i16, IrError> {
        Ok(i16::from_le_bytes(self.chunk::<2>()?))
    }
    fn u32(&mut self) -> Result<u32, IrError> {
        Ok(u32::from_le_bytes(self.chunk::<4>()?))
    }
    fn i32(&mut self) -> Result<i32, IrError> {
        Ok(i32::from_le_bytes(self.chunk::<4>()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(data: Vec<u8>) -> Record {
        Record { tag: tag::PARA_SHAPE, level: 1, data }
    }

    fn sample() -> ParaShape {
        ParaShape {
            attribute: 0x0700_0001, // align=1 (Left) + heading_level=7
            left_margin: 100,
            right_margin: 200,
            indent: -50,
            top_space: 10,
            bottom_space: 20,
            line_space_legacy: 160,
            tab_def_id: 0,
            heading_id: 0,
            border_fill_id: 0,
            left_border_space: 5,
            right_border_space: 5,
            top_border_space: -3,
            bottom_border_space: -3,
            attribute2: Some(0),
            attribute3: Some(0),
            // attribute bits 0–1 = 1 (FIXED line spacing kind), so
            // the parser derives kind=1 on read-back. Keep the
            // sample consistent with that.
            line_spacing_kind: Some(1),
            line_spacing: Some(160),
        }
    }

    #[test]
    fn full_58_byte_roundtrips() {
        let ps = sample();
        let bytes = emit(&ps);
        assert_eq!(bytes.len(), WITH_LINE_SPACING);
        assert_eq!(parse(&rec(bytes)).unwrap(), ps);
    }

    #[test]
    fn minimal_42_byte_roundtrips() {
        let mut ps = sample();
        ps.attribute2 = None;
        ps.attribute3 = None;
        ps.line_spacing_kind = None;
        ps.line_spacing = None;
        let bytes = emit(&ps);
        assert_eq!(bytes.len(), BASE);
        assert_eq!(parse(&rec(bytes)).unwrap(), ps);
    }

    #[test]
    fn attribute_accessors() {
        let mut ps = ParaShape::default();
        ps.attribute = 0x0000_0002;
        assert_eq!(ps.align(), 2);
        ps.attribute = 1 << 6;
        assert!(ps.snap_to_grid());
        ps.attribute = 1 << 16;
        assert!(ps.keep_with_next());
        ps.attribute = 1 << 18;
        assert!(ps.page_break_before());
        ps.attribute = 3 << 24;
        assert_eq!(ps.heading_level(), 3);
    }

    #[test]
    fn wrong_tag_errors() {
        let r = Record { tag: tag::CHAR_SHAPE, level: 0, data: vec![0; BASE] };
        assert!(parse(&r).is_err());
    }

    #[test]
    fn short_data_errors() {
        let r = rec(vec![0; BASE - 1]);
        assert!(parse(&r).is_err());
    }
}
