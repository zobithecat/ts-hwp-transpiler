//! CharShape (tag 0x0015) record codec.
//!
//! Layout (hwplib `CharShapeReader`):
//!
//!   u16[7]  font_ids         // script slots: hangul/latin/hanja/jp/other/sym/user
//!   u8[7]   ratios           // 50..200
//!   i8[7]   char_spacings    // -50..+50
//!   u8[7]   rel_sizes        // 10..250
//!   i8[7]   char_offsets     // baseline shift
//!   i32     base_size        // 1/100 pt
//!   u32     attr             // bitfield (italic/bold/underline/...)
//!   i8      shadow_offset_x
//!   i8      shadow_offset_y
//!   u32     color            // RGBA
//!   u32     underline_color
//!   u32     shade_color
//!   u32     shadow_color
//!   [opt]   u16  border_fill_id    (HWP 5.0.2.1+, when record size >= 70)
//!   [opt]   u32  strike_color      (HWP 5.0.3.0+, when record size >= 74)
//!
//! Minimum record size: 68 bytes. Optional tail handled by length.

use hwp_transpiler_core::ir::{CharShape, IrError, Record};

use super::doc_info::tag;

const MIN_SIZE: usize = 14 + 7 * 4 + 4 + 4 + 2 + 16; // 68
const WITH_BORDER_FILL_ID: usize = MIN_SIZE + 2; // 70
const WITH_STRIKE_COLOR: usize = WITH_BORDER_FILL_ID + 4; // 74

pub fn parse(rec: &Record) -> Result<CharShape, IrError> {
    if rec.tag != tag::CHAR_SHAPE {
        return Err(IrError::Invalid(format!(
            "expected CharShape (0x{:04X}), got 0x{:04X}",
            tag::CHAR_SHAPE,
            rec.tag
        )));
    }
    if rec.data.len() < MIN_SIZE {
        return Err(IrError::Invalid(format!(
            "CharShape too short: {} < {MIN_SIZE}",
            rec.data.len()
        )));
    }

    let mut c = Cur::new(&rec.data);
    let mut font_ids = [0u16; 7];
    for v in &mut font_ids { *v = c.u16()?; }
    let mut ratios = [0u8; 7];
    for v in &mut ratios { *v = c.u8()?; }
    let mut char_spacings = [0i8; 7];
    for v in &mut char_spacings { *v = c.i8()?; }
    let mut rel_sizes = [0u8; 7];
    for v in &mut rel_sizes { *v = c.u8()?; }
    let mut char_offsets = [0i8; 7];
    for v in &mut char_offsets { *v = c.i8()?; }

    let base_size = c.i32()?;
    let attr = c.u32()?;
    let shadow_offset_x = c.i8()?;
    let shadow_offset_y = c.i8()?;
    let color = c.u32()?;
    let underline_color = c.u32()?;
    let shade_color = c.u32()?;
    let shadow_color = c.u32()?;

    let border_fill_id = if rec.data.len() >= WITH_BORDER_FILL_ID {
        Some(c.u16()?)
    } else {
        None
    };
    let strike_color = if rec.data.len() >= WITH_STRIKE_COLOR {
        Some(c.u32()?)
    } else {
        None
    };

    Ok(CharShape {
        font_ids,
        ratios,
        char_spacings,
        rel_sizes,
        char_offsets,
        base_size,
        attr,
        shadow_offset_x,
        shadow_offset_y,
        color,
        underline_color,
        shade_color,
        shadow_color,
        border_fill_id,
        strike_color,
    })
}

pub fn emit(cs: &CharShape) -> Vec<u8> {
    let mut out = Vec::with_capacity(WITH_STRIKE_COLOR);
    for v in cs.font_ids { out.extend_from_slice(&v.to_le_bytes()); }
    for v in cs.ratios { out.push(v); }
    for v in cs.char_spacings { out.push(v as u8); }
    for v in cs.rel_sizes { out.push(v); }
    for v in cs.char_offsets { out.push(v as u8); }
    out.extend_from_slice(&cs.base_size.to_le_bytes());
    out.extend_from_slice(&cs.attr.to_le_bytes());
    out.push(cs.shadow_offset_x as u8);
    out.push(cs.shadow_offset_y as u8);
    out.extend_from_slice(&cs.color.to_le_bytes());
    out.extend_from_slice(&cs.underline_color.to_le_bytes());
    out.extend_from_slice(&cs.shade_color.to_le_bytes());
    out.extend_from_slice(&cs.shadow_color.to_le_bytes());
    if let Some(id) = cs.border_fill_id {
        out.extend_from_slice(&id.to_le_bytes());
    }
    if let Some(sc) = cs.strike_color {
        out.extend_from_slice(&sc.to_le_bytes());
    }
    out
}

struct Cur<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cur<'a> {
    fn new(bytes: &'a [u8]) -> Self { Self { bytes, pos: 0 } }

    fn u8(&mut self) -> Result<u8, IrError> {
        let b = self.bytes.get(self.pos).copied().ok_or_else(|| {
            IrError::Invalid(format!("CharShape: u8 OOB at {}", self.pos))
        })?;
        self.pos += 1;
        Ok(b)
    }

    fn i8(&mut self) -> Result<i8, IrError> {
        self.u8().map(|b| b as i8)
    }

    fn u16(&mut self) -> Result<u16, IrError> {
        Ok(u16::from_le_bytes(self.chunk::<2>()?))
    }

    fn u32(&mut self) -> Result<u32, IrError> {
        Ok(u32::from_le_bytes(self.chunk::<4>()?))
    }

    fn i32(&mut self) -> Result<i32, IrError> {
        Ok(i32::from_le_bytes(self.chunk::<4>()?))
    }

    fn chunk<const N: usize>(&mut self) -> Result<[u8; N], IrError> {
        if self.pos + N > self.bytes.len() {
            return Err(IrError::Invalid(format!(
                "CharShape: {N}-byte OOB at {}/{}",
                self.pos,
                self.bytes.len()
            )));
        }
        let mut buf = [0u8; N];
        buf.copy_from_slice(&self.bytes[self.pos..self.pos + N]);
        self.pos += N;
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(data: Vec<u8>) -> Record {
        Record { tag: tag::CHAR_SHAPE, level: 1, data }
    }

    fn sample() -> CharShape {
        CharShape {
            font_ids: [0, 0, 0, 0, 0, 0, 0],
            ratios: [100; 7],
            char_spacings: [0; 7],
            rel_sizes: [100; 7],
            char_offsets: [0; 7],
            base_size: 1000,
            attr: 0x0000_0003, // italic + bold
            shadow_offset_x: 10,
            shadow_offset_y: 10,
            color: 0xFF_00_00_00,
            underline_color: 0xFF_FF_00_00,
            shade_color: 0,
            shadow_color: 0x80_80_80_80,
            border_fill_id: Some(0),
            strike_color: Some(0xFF_00_FF_00),
        }
    }

    #[test]
    fn full_74_byte_roundtrips() {
        let cs = sample();
        let bytes = emit(&cs);
        assert_eq!(bytes.len(), WITH_STRIKE_COLOR);
        assert_eq!(parse(&rec(bytes)).unwrap(), cs);
    }

    #[test]
    fn minimal_68_byte_roundtrips() {
        let mut cs = sample();
        cs.border_fill_id = None;
        cs.strike_color = None;
        let bytes = emit(&cs);
        assert_eq!(bytes.len(), MIN_SIZE);
        assert_eq!(parse(&rec(bytes)).unwrap(), cs);
    }

    #[test]
    fn with_border_fill_id_only_70_byte() {
        let mut cs = sample();
        cs.strike_color = None;
        let bytes = emit(&cs);
        assert_eq!(bytes.len(), WITH_BORDER_FILL_ID);
        assert_eq!(parse(&rec(bytes)).unwrap(), cs);
    }

    #[test]
    fn attribute_bit_accessors() {
        let mut cs = sample();
        cs.attr = 0x0000_0000;
        assert!(!cs.italic() && !cs.bold());
        cs.attr = 0x0000_0003;
        assert!(cs.italic() && cs.bold());
        cs.attr = 1 << 17; // script = 1 (superscript)
        assert!(cs.is_superscript() && !cs.is_subscript());
        cs.attr = 2 << 17; // script = 2 (subscript)
        assert!(!cs.is_superscript() && cs.is_subscript());
        cs.attr = 1 << 21;
        assert!(cs.strike());
    }

    #[test]
    fn negative_spacings_and_offsets() {
        let mut cs = sample();
        cs.char_spacings = [-50; 7];
        cs.char_offsets = [-40; 7];
        cs.shadow_offset_x = -20;
        cs.shadow_offset_y = -15;
        let bytes = emit(&cs);
        assert_eq!(parse(&rec(bytes)).unwrap(), cs);
    }

    #[test]
    fn wrong_tag_errors() {
        let r = Record { tag: tag::FACE_NAME, level: 0, data: vec![0; MIN_SIZE] };
        assert!(parse(&r).is_err());
    }

    #[test]
    fn short_data_errors() {
        let r = rec(vec![0; MIN_SIZE - 1]);
        assert!(parse(&r).is_err());
    }
}
