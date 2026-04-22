//! PARA_CHAR_SHAPE (tag 0x0044) — list of `CharShapeRun` entries.
//!
//! Each entry = 8 bytes: u32 start, u32 char_shape_id. The record data
//! length must be a multiple of 8.

use hwp_transpiler_core::ir::{CharShapeRun, IrError, Record};

use super::body_text::tag;

const ENTRY: usize = 8;

pub fn parse(rec: &Record) -> Result<Vec<CharShapeRun>, IrError> {
    if rec.tag != tag::PARA_CHAR_SHAPE {
        return Err(IrError::Invalid(format!(
            "expected PARA_CHAR_SHAPE (0x{:04X}), got 0x{:04X}",
            tag::PARA_CHAR_SHAPE,
            rec.tag
        )));
    }
    if rec.data.len() % ENTRY != 0 {
        return Err(IrError::Invalid(format!(
            "PARA_CHAR_SHAPE not {ENTRY}B-aligned: {} bytes",
            rec.data.len()
        )));
    }
    Ok(rec
        .data
        .chunks_exact(ENTRY)
        .map(|c| CharShapeRun {
            start: u32::from_le_bytes([c[0], c[1], c[2], c[3]]),
            char_shape_id: u32::from_le_bytes([c[4], c[5], c[6], c[7]]),
        })
        .collect())
}

pub fn emit(runs: &[CharShapeRun]) -> Vec<u8> {
    let mut out = Vec::with_capacity(runs.len() * ENTRY);
    for r in runs {
        out.extend_from_slice(&r.start.to_le_bytes());
        out.extend_from_slice(&r.char_shape_id.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(data: Vec<u8>) -> Record {
        Record { tag: tag::PARA_CHAR_SHAPE, level: 1, data }
    }

    #[test]
    fn roundtrip() {
        let runs = vec![
            CharShapeRun { start: 0, char_shape_id: 1 },
            CharShapeRun { start: 10, char_shape_id: 3 },
            CharShapeRun { start: 25, char_shape_id: 1 },
        ];
        let bytes = emit(&runs);
        assert_eq!(bytes.len(), 3 * ENTRY);
        assert_eq!(parse(&rec(bytes)).unwrap(), runs);
    }

    #[test]
    fn empty_ok() {
        assert_eq!(parse(&rec(vec![])).unwrap(), Vec::<CharShapeRun>::new());
    }

    #[test]
    fn misaligned_errors() {
        assert!(parse(&rec(vec![0; 7])).is_err());
    }

    #[test]
    fn wrong_tag_errors() {
        let r = Record { tag: tag::PARA_HEADER, level: 0, data: vec![] };
        assert!(parse(&r).is_err());
    }
}
