//! BinData (DocInfo tag 0x0012) record codec.
//!
//! Layout (from hwplib `reader/docinfo/ForBinData.java`):
//!
//!   property:        u16  (low nibble = type)
//!   if type == Link (0):
//!     absolute_path: UTF-16LE (u16 code-unit prefix)
//!     relative_path: UTF-16LE
//!   if type == Embedding (1) or Storage (2):
//!     bin_data_id:   u16   (matches `/BinData/BIN<id>` storage stream)
//!     extension:     UTF-16LE

use hwp_transpiler_core::ir::{BinData, IrError, Record};

use super::doc_info::tag;

pub fn parse(rec: &Record) -> Result<BinData, IrError> {
    if rec.tag != tag::BIN_DATA {
        return Err(IrError::Invalid(format!(
            "expected BinData (0x{:04X}), got 0x{:04X}",
            tag::BIN_DATA,
            rec.tag
        )));
    }
    let mut c = Cursor::new(&rec.data);
    let property = c.u16_le()?;
    let mut bd = BinData {
        property,
        ..BinData::default()
    };
    match property & 0x000F {
        BinData::TYPE_LINK => {
            bd.absolute_path = Some(c.utf16le()?);
            bd.relative_path = Some(c.utf16le()?);
        }
        BinData::TYPE_EMBEDDING | BinData::TYPE_STORAGE => {
            bd.bin_data_id = Some(c.u16_le()?);
            bd.extension = Some(c.utf16le()?);
        }
        other => {
            return Err(IrError::Invalid(format!(
                "BinData unknown type {other:#x} (property={property:#06x})"
            )));
        }
    }
    Ok(bd)
}

pub fn emit(bd: &BinData) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&bd.property.to_le_bytes());
    match bd.property & 0x000F {
        BinData::TYPE_LINK => {
            write_utf16le(&mut out, bd.absolute_path.as_deref().unwrap_or(""));
            write_utf16le(&mut out, bd.relative_path.as_deref().unwrap_or(""));
        }
        BinData::TYPE_EMBEDDING | BinData::TYPE_STORAGE => {
            out.extend_from_slice(&bd.bin_data_id.unwrap_or(0).to_le_bytes());
            write_utf16le(&mut out, bd.extension.as_deref().unwrap_or(""));
        }
        _ => {
            // Unknown type: emit only the property word. The reader rejects
            // it on the next round, which is correct for a value we
            // couldn't have produced ourselves.
        }
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

    fn u16_le(&mut self) -> Result<u16, IrError> {
        if self.pos + 2 > self.bytes.len() {
            return Err(IrError::Invalid(format!(
                "BinData read u16 at {}/{}",
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
                "BinData utf16 truncated: code_units={code_units} need={byte_len} remaining={}",
                self.bytes.len() - self.pos
            )));
        }
        let units: Vec<u16> = self.bytes[self.pos..self.pos + byte_len]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        self.pos += byte_len;
        String::from_utf16(&units)
            .map_err(|e| IrError::Invalid(format!("BinData invalid UTF-16: {e}")))
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
        Record { tag: tag::BIN_DATA, level: 0, data }
    }

    fn roundtrip(bd: BinData) {
        let bytes = emit(&bd);
        let parsed = parse(&rec(bytes)).expect("parse");
        assert_eq!(parsed, bd);
    }

    #[test]
    fn embedding_roundtrips() {
        roundtrip(BinData {
            property: BinData::TYPE_EMBEDDING,
            bin_data_id: Some(1),
            extension: Some("png".into()),
            ..BinData::default()
        });
    }

    #[test]
    fn link_roundtrips() {
        roundtrip(BinData {
            property: BinData::TYPE_LINK,
            absolute_path: Some("/tmp/cat.png".into()),
            relative_path: Some("../images/cat.png".into()),
            ..BinData::default()
        });
    }

    #[test]
    fn storage_roundtrips() {
        roundtrip(BinData {
            property: BinData::TYPE_STORAGE,
            bin_data_id: Some(7),
            extension: Some("ole".into()),
            ..BinData::default()
        });
    }

    #[test]
    fn embedding_with_korean_extension() {
        roundtrip(BinData {
            property: BinData::TYPE_EMBEDDING,
            bin_data_id: Some(2),
            extension: Some("그림".into()),
            ..BinData::default()
        });
    }

    #[test]
    fn property_high_bits_are_preserved() {
        // Bits 4..=15 carry compression/status/reserved info we don't
        // decode — must round-trip byte-equal.
        roundtrip(BinData {
            property: 0x1230 | BinData::TYPE_EMBEDDING,
            bin_data_id: Some(99),
            extension: Some("jpg".into()),
            ..BinData::default()
        });
    }

    #[test]
    fn wrong_tag_errors() {
        let r = Record { tag: tag::DOCUMENT_PROPERTIES, level: 0, data: vec![0; 8] };
        assert!(parse(&r).is_err());
    }

    #[test]
    fn truncated_property_errors() {
        assert!(parse(&rec(vec![0])).is_err());
    }

    #[test]
    fn unknown_type_errors() {
        let mut data = vec![0u8; 2];
        data[0] = 0x05; // type = 5 → out of {Link, Embedding, Storage}
        assert!(parse(&rec(data)).is_err());
    }
}
