//! Style (tag 0x001A) record codec.
//!
//! Layout (hwplib `StyleReader`):
//!
//!   name:          UTF-16LE, u16 LE char-count prefix
//!   english_name:  UTF-16LE, u16 LE char-count prefix
//!   properties:    u8    (bit 0..=2 style type: 0=paragraph, 1=character)
//!   next_style_id: u8
//!   lang_id:       i16 LE
//!   para_shape_id: u16 LE
//!   char_shape_id: u16 LE

use hwp_transpiler_core::ir::{IrError, Record, Style};

use super::doc_info::tag;

const FIXED_TAIL: usize = 1 + 1 + 2 + 2 + 2; // 8 bytes after both strings
const MIN_SIZE: usize = 2 /* empty name */ + 2 /* empty eng */ + FIXED_TAIL;

pub fn parse(rec: &Record) -> Result<Style, IrError> {
    if rec.tag != tag::STYLE {
        return Err(IrError::Invalid(format!(
            "expected Style (0x{:04X}), got 0x{:04X}",
            tag::STYLE,
            rec.tag
        )));
    }
    if rec.data.len() < MIN_SIZE {
        return Err(IrError::Invalid(format!(
            "Style too short: {} < {MIN_SIZE}",
            rec.data.len()
        )));
    }
    let mut c = Cur::new(&rec.data);
    let name = c.utf16le()?;
    let english_name = c.utf16le()?;
    let properties = c.u8()?;
    let next_style_id = c.u8()?;
    let lang_id = c.i16()?;
    let para_shape_id = c.u16()?;
    let char_shape_id = c.u16()?;

    Ok(Style {
        name,
        english_name,
        properties,
        next_style_id,
        lang_id,
        para_shape_id,
        char_shape_id,
    })
}

pub fn emit(s: &Style) -> Vec<u8> {
    let mut out = Vec::new();
    write_utf16le(&mut out, &s.name);
    write_utf16le(&mut out, &s.english_name);
    out.push(s.properties);
    out.push(s.next_style_id);
    out.extend_from_slice(&s.lang_id.to_le_bytes());
    out.extend_from_slice(&s.para_shape_id.to_le_bytes());
    out.extend_from_slice(&s.char_shape_id.to_le_bytes());
    out
}

fn write_utf16le(out: &mut Vec<u8>, s: &str) {
    let units: Vec<u16> = s.encode_utf16().collect();
    out.extend_from_slice(&(units.len() as u16).to_le_bytes());
    for u in units {
        out.extend_from_slice(&u.to_le_bytes());
    }
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
                "Style: {N}-byte OOB at {}/{}",
                self.pos,
                self.bytes.len()
            )));
        }
        let mut b = [0u8; N];
        b.copy_from_slice(&self.bytes[self.pos..self.pos + N]);
        self.pos += N;
        Ok(b)
    }

    fn u8(&mut self) -> Result<u8, IrError> {
        Ok(self.chunk::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16, IrError> {
        Ok(u16::from_le_bytes(self.chunk::<2>()?))
    }
    fn i16(&mut self) -> Result<i16, IrError> {
        Ok(i16::from_le_bytes(self.chunk::<2>()?))
    }

    fn utf16le(&mut self) -> Result<String, IrError> {
        let units = self.u16()? as usize;
        let bytes_needed = units * 2;
        if self.pos + bytes_needed > self.bytes.len() {
            return Err(IrError::Invalid(format!(
                "Style UTF-16 truncated: units={units} need={bytes_needed} remaining={}",
                self.bytes.len() - self.pos
            )));
        }
        let u16s: Vec<u16> = self.bytes[self.pos..self.pos + bytes_needed]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        self.pos += bytes_needed;
        String::from_utf16(&u16s)
            .map_err(|e| IrError::Invalid(format!("invalid UTF-16 in Style: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(data: Vec<u8>) -> Record {
        Record { tag: tag::STYLE, level: 1, data }
    }

    fn sample() -> Style {
        Style {
            name: "본문".into(),
            english_name: "Normal".into(),
            properties: 0,
            next_style_id: 0,
            lang_id: 0x0412, // Korean
            para_shape_id: 3,
            char_shape_id: 5,
        }
    }

    #[test]
    fn roundtrip() {
        let s = sample();
        let bytes = emit(&s);
        assert_eq!(parse(&rec(bytes)).unwrap(), s);
    }

    #[test]
    fn empty_strings() {
        let s = Style {
            name: String::new(),
            english_name: String::new(),
            properties: 1,
            next_style_id: 0,
            lang_id: 0,
            para_shape_id: 0,
            char_shape_id: 0,
        };
        let bytes = emit(&s);
        assert_eq!(bytes.len(), MIN_SIZE);
        assert_eq!(parse(&rec(bytes)).unwrap(), s);
    }

    #[test]
    fn character_style_properties() {
        let mut s = sample();
        s.properties = 1;
        let bytes = emit(&s);
        assert_eq!(parse(&rec(bytes)).unwrap(), s);
    }

    #[test]
    fn wrong_tag_errors() {
        let r = Record { tag: tag::PARA_SHAPE, level: 0, data: vec![0; MIN_SIZE] };
        assert!(parse(&r).is_err());
    }

    #[test]
    fn short_data_errors() {
        let r = rec(vec![0; MIN_SIZE - 1]);
        assert!(parse(&r).is_err());
    }

    #[test]
    fn truncated_second_string_errors() {
        // name len=2 + 4 bytes "aa" + eng len=5 but no bytes
        let bytes = vec![2, 0, 0x61, 0x00, 0x61, 0x00, 5, 0];
        let r = rec(bytes);
        assert!(parse(&r).is_err());
    }
}
