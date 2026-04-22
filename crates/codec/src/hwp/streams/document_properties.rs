//! DocumentProperties (tag 0x0010) record codec.
//!
//! Exactly one record per document. HWP5 revisions disagree on what lives
//! *after* the mandatory 22-byte core:
//!
//!   Core (22 B, stable):
//!     u16  section_count
//!     u16  page_start_number
//!     u16  footnote_start_number
//!     u16  endnote_start_number
//!     u16  picture_start_number
//!     u16  table_start_number
//!     u16  equation_start_number
//!     u32  total_character_count
//!     u32  total_page_count
//!
//!   Tail (variable):
//!     - Modern blank docs ship 4 bytes here (single u32, purpose TBD).
//!     - Older docs ship 16 bytes (`[u32; 4]` caret position).
//!     - Versions beyond may extend further.
//!
//! We preserve the tail verbatim to round-trip losslessly, and expose a
//! decoder (`DocProperties::caret_position`) for the common form.

use hwp_transpiler_core::ir::{DocProperties, IrError, Record};

use super::doc_info::tag;

pub const CORE_SIZE: usize = 7 * 2 + 2 * 4; // 22

pub fn parse(rec: &Record) -> Result<DocProperties, IrError> {
    if rec.tag != tag::DOCUMENT_PROPERTIES {
        return Err(IrError::Invalid(format!(
            "expected DocumentProperties (0x{:04X}), got 0x{:04X}",
            tag::DOCUMENT_PROPERTIES,
            rec.tag
        )));
    }
    if rec.data.len() < CORE_SIZE {
        return Err(IrError::Invalid(format!(
            "DocumentProperties core too short: {} < {CORE_SIZE}",
            rec.data.len()
        )));
    }
    let mut c = Cur::new(&rec.data);
    let section_count = c.u16()?;
    let page_start_number = c.u16()?;
    let footnote_start_number = c.u16()?;
    let endnote_start_number = c.u16()?;
    let picture_start_number = c.u16()?;
    let table_start_number = c.u16()?;
    let equation_start_number = c.u16()?;
    let total_character_count = c.u32()?;
    let total_page_count = c.u32()?;
    let tail = rec.data[CORE_SIZE..].to_vec();

    Ok(DocProperties {
        section_count,
        page_start_number,
        footnote_start_number,
        endnote_start_number,
        picture_start_number,
        table_start_number,
        equation_start_number,
        total_character_count,
        total_page_count,
        tail,
    })
}

pub fn emit(p: &DocProperties) -> Vec<u8> {
    let mut out = Vec::with_capacity(CORE_SIZE + p.tail.len());
    out.extend_from_slice(&p.section_count.to_le_bytes());
    out.extend_from_slice(&p.page_start_number.to_le_bytes());
    out.extend_from_slice(&p.footnote_start_number.to_le_bytes());
    out.extend_from_slice(&p.endnote_start_number.to_le_bytes());
    out.extend_from_slice(&p.picture_start_number.to_le_bytes());
    out.extend_from_slice(&p.table_start_number.to_le_bytes());
    out.extend_from_slice(&p.equation_start_number.to_le_bytes());
    out.extend_from_slice(&p.total_character_count.to_le_bytes());
    out.extend_from_slice(&p.total_page_count.to_le_bytes());
    out.extend_from_slice(&p.tail);
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
                "DocumentProperties: {N}-byte OOB at {}",
                self.pos
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
    fn u32(&mut self) -> Result<u32, IrError> {
        Ok(u32::from_le_bytes(self.chunk::<4>()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(data: Vec<u8>) -> Record {
        Record { tag: tag::DOCUMENT_PROPERTIES, level: 0, data }
    }

    #[test]
    fn roundtrip_with_16byte_caret_tail() {
        let p = DocProperties {
            section_count: 1,
            page_start_number: 1,
            footnote_start_number: 1,
            endnote_start_number: 1,
            picture_start_number: 1,
            table_start_number: 1,
            equation_start_number: 1,
            total_character_count: 42,
            total_page_count: 3,
            tail: (0u32..4).flat_map(|i| i.to_le_bytes()).collect(),
        };
        let bytes = emit(&p);
        assert_eq!(bytes.len(), CORE_SIZE + 16);
        let parsed = parse(&rec(bytes)).unwrap();
        assert_eq!(parsed, p);
        assert_eq!(parsed.caret_position(), Some([0, 1, 2, 3]));
    }

    #[test]
    fn roundtrip_with_4byte_tail() {
        let p = DocProperties {
            section_count: 1,
            page_start_number: 1,
            footnote_start_number: 1,
            endnote_start_number: 1,
            picture_start_number: 1,
            table_start_number: 1,
            equation_start_number: 1,
            total_character_count: 0,
            total_page_count: 1,
            tail: vec![0, 0, 0, 0],
        };
        let bytes = emit(&p);
        assert_eq!(bytes.len(), CORE_SIZE + 4);
        let parsed = parse(&rec(bytes)).unwrap();
        assert_eq!(parsed, p);
        assert_eq!(parsed.caret_position(), None); // tail too short
    }

    #[test]
    fn empty_tail_ok() {
        let mut p = DocProperties::default();
        p.section_count = 0;
        let bytes = emit(&p);
        assert_eq!(bytes.len(), CORE_SIZE);
        assert_eq!(parse(&rec(bytes)).unwrap(), p);
    }

    #[test]
    fn wrong_tag_errors() {
        let r = Record { tag: tag::STYLE, level: 0, data: vec![0; CORE_SIZE] };
        assert!(parse(&r).is_err());
    }

    #[test]
    fn short_core_errors() {
        let r = rec(vec![0; CORE_SIZE - 1]);
        assert!(parse(&r).is_err());
    }
}
