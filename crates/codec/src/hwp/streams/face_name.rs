//! FaceName (tag 0x0013) record codec.
//!
//! Layout (hwplib `FaceNameReader` canonical):
//!
//!   properties:  u8
//!   name:        UTF-16LE, u16-LE char-count prefix
//!   if props & 0x80:  subst_kind (u8) + subst_name (UTF-16LE prefixed)
//!   if props & 0x40:  font_type_info (10 bytes, Panose 1.0)
//!   if props & 0x20:  default_name (UTF-16LE prefixed)
//!
//! The u16 length prefix counts UTF-16 code units (not bytes, not chars
//! for supplementary-plane). Surrogate pairs stay as two u16s.

use hwp_transpiler_core::ir::{FontFace, FontTypeInfo, IrError, Record, SubstituteFont};

use super::doc_info::tag;

pub const FLAG_HAS_SUBSTITUTE: u8 = 0x80;
pub const FLAG_HAS_TYPE_INFO: u8 = 0x40;
pub const FLAG_HAS_DEFAULT_NAME: u8 = 0x20;

pub fn parse(rec: &Record) -> Result<FontFace, IrError> {
    if rec.tag != tag::FACE_NAME {
        return Err(IrError::Invalid(format!(
            "expected FaceName (0x{:04X}), got 0x{:04X}",
            tag::FACE_NAME,
            rec.tag
        )));
    }
    let mut c = Cursor::new(&rec.data);
    let properties = c.u8()?;
    let name = c.utf16le()?;

    let substitute = if properties & FLAG_HAS_SUBSTITUTE != 0 {
        let kind = c.u8()?;
        let name = c.utf16le()?;
        Some(SubstituteFont { kind, name })
    } else {
        None
    };

    let type_info = if properties & FLAG_HAS_TYPE_INFO != 0 {
        Some(FontTypeInfo {
            family: c.u8()?,
            serif: c.u8()?,
            weight: c.u8()?,
            proportion: c.u8()?,
            contrast: c.u8()?,
            stroke_variation: c.u8()?,
            arm_style: c.u8()?,
            letterform: c.u8()?,
            midline: c.u8()?,
            x_height: c.u8()?,
        })
    } else {
        None
    };

    let default_name = if properties & FLAG_HAS_DEFAULT_NAME != 0 {
        Some(c.utf16le()?)
    } else {
        None
    };

    Ok(FontFace {
        properties,
        name,
        substitute,
        type_info,
        default_name,
    })
}

pub fn emit(face: &FontFace) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(face.properties);
    write_utf16le(&mut out, &face.name);
    if let Some(s) = &face.substitute {
        out.push(s.kind);
        write_utf16le(&mut out, &s.name);
    }
    if let Some(ti) = &face.type_info {
        out.extend_from_slice(&[
            ti.family,
            ti.serif,
            ti.weight,
            ti.proportion,
            ti.contrast,
            ti.stroke_variation,
            ti.arm_style,
            ti.letterform,
            ti.midline,
            ti.x_height,
        ]);
    }
    if let Some(dn) = &face.default_name {
        write_utf16le(&mut out, dn);
    }
    out
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn u8(&mut self) -> Result<u8, IrError> {
        let b = self.bytes.get(self.pos).copied().ok_or_else(|| {
            IrError::Invalid(format!(
                "read u8 at {}/{}",
                self.pos,
                self.bytes.len()
            ))
        })?;
        self.pos += 1;
        Ok(b)
    }

    fn u16_le(&mut self) -> Result<u16, IrError> {
        if self.pos + 2 > self.bytes.len() {
            return Err(IrError::Invalid(format!(
                "read u16 at {}/{}",
                self.pos,
                self.bytes.len()
            )));
        }
        let v = u16::from_le_bytes([self.bytes[self.pos], self.bytes[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }

    fn utf16le(&mut self) -> Result<String, IrError> {
        let code_units = self.u16_le()? as usize;
        let byte_len = code_units * 2;
        if self.pos + byte_len > self.bytes.len() {
            return Err(IrError::Invalid(format!(
                "utf16 truncated: code_units={} need={} remaining={}",
                code_units,
                byte_len,
                self.bytes.len() - self.pos
            )));
        }
        let units: Vec<u16> = self.bytes[self.pos..self.pos + byte_len]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        self.pos += byte_len;
        String::from_utf16(&units).map_err(|e| IrError::Invalid(format!("invalid UTF-16: {e}")))
    }
}

fn write_utf16le(out: &mut Vec<u8>, s: &str) {
    let units: Vec<u16> = s.encode_utf16().collect();
    out.extend_from_slice(&(units.len() as u16).to_le_bytes());
    for u in units {
        out.extend_from_slice(&u.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(data: Vec<u8>) -> Record {
        Record { tag: tag::FACE_NAME, level: 1, data }
    }

    fn roundtrip(face: FontFace) {
        let bytes = emit(&face);
        let parsed = parse(&rec(bytes)).expect("parse");
        assert_eq!(parsed, face);
    }

    #[test]
    fn bare_name() {
        roundtrip(FontFace {
            properties: 0,
            name: "함초롬바탕".into(),
            substitute: None,
            type_info: None,
            default_name: None,
        });
    }

    #[test]
    fn with_substitute() {
        roundtrip(FontFace {
            properties: FLAG_HAS_SUBSTITUTE,
            name: "함초롬바탕".into(),
            substitute: Some(SubstituteFont {
                kind: 1,
                name: "NanumMyeongjo".into(),
            }),
            type_info: None,
            default_name: None,
        });
    }

    #[test]
    fn with_type_info_only() {
        roundtrip(FontFace {
            properties: FLAG_HAS_TYPE_INFO,
            name: "Test".into(),
            substitute: None,
            type_info: Some(FontTypeInfo {
                family: 2,
                serif: 3,
                weight: 5,
                proportion: 4,
                contrast: 2,
                stroke_variation: 2,
                arm_style: 5,
                letterform: 3,
                midline: 0,
                x_height: 4,
            }),
            default_name: None,
        });
    }

    #[test]
    fn all_optionals() {
        roundtrip(FontFace {
            properties: FLAG_HAS_SUBSTITUTE | FLAG_HAS_TYPE_INFO | FLAG_HAS_DEFAULT_NAME,
            name: "Main".into(),
            substitute: Some(SubstituteFont { kind: 2, name: "Sub".into() }),
            type_info: Some(FontTypeInfo {
                family: 1, serif: 1, weight: 1, proportion: 1, contrast: 1,
                stroke_variation: 1, arm_style: 1, letterform: 1, midline: 1, x_height: 1,
            }),
            default_name: Some("Default".into()),
        });
    }

    #[test]
    fn empty_name_roundtrips() {
        roundtrip(FontFace {
            properties: 0,
            name: String::new(),
            substitute: None,
            type_info: None,
            default_name: None,
        });
    }

    #[test]
    fn wrong_tag_errors() {
        let rec = Record { tag: 0x10, level: 0, data: vec![0; 8] };
        assert!(parse(&rec).is_err());
    }

    #[test]
    fn truncated_name_errors() {
        // properties + declared len=10 but only 4 bytes follow
        let rec = rec(vec![0, 10, 0, 0xAB, 0xCD, 0xEF, 0x01]);
        assert!(parse(&rec).is_err());
    }
}
