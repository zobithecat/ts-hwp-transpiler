//! BorderFill (tag 0x0014) record codec.
//!
//! Layout (hwplib `BorderFillReader`):
//!
//!   attribute: u16 LE
//!   left  : Border (6 B)
//!   right : Border
//!   top   : Border
//!   bottom: Border
//!   diag  : Border
//!   fill:
//!     kind: u32 LE        (bit 0=Color, 1=Gradation, 2=Image)
//!     body: remaining     (decoded structurally in later rounds)
//!
//! Border = { kind: u8, width: u8, color: u32 LE }.

use hwp_transpiler_core::ir::{Border, BorderFill, Fill, IrError, Record};

use super::doc_info::tag;

const HEADER_SIZE: usize = 2 /* attribute */ + 6 * 5 /* 5 borders */ + 4 /* fill.kind */;

pub fn parse(rec: &Record) -> Result<BorderFill, IrError> {
    if rec.tag != tag::BORDER_FILL {
        return Err(IrError::Invalid(format!(
            "expected BorderFill (0x{:04X}), got 0x{:04X}",
            tag::BORDER_FILL,
            rec.tag
        )));
    }
    if rec.data.len() < HEADER_SIZE {
        return Err(IrError::Invalid(format!(
            "BorderFill too short: {} < {HEADER_SIZE}",
            rec.data.len()
        )));
    }
    let mut c = Cur::new(&rec.data);
    let attribute = c.u16()?;
    let borders = [c.border()?, c.border()?, c.border()?, c.border()?];
    let diagonal = c.border()?;
    let fill_kind = c.u32()?;
    let body = c.remaining().to_vec();

    Ok(BorderFill {
        attribute,
        borders,
        diagonal,
        fill: Fill { kind: fill_kind, body },
    })
}

pub fn emit(bf: &BorderFill) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_SIZE + bf.fill.body.len());
    out.extend_from_slice(&bf.attribute.to_le_bytes());
    for b in &bf.borders {
        emit_border(&mut out, b);
    }
    emit_border(&mut out, &bf.diagonal);
    out.extend_from_slice(&bf.fill.kind.to_le_bytes());
    out.extend_from_slice(&bf.fill.body);
    out
}

fn emit_border(out: &mut Vec<u8>, b: &Border) {
    out.push(b.kind);
    out.push(b.width);
    out.extend_from_slice(&b.color.to_le_bytes());
}

struct Cur<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cur<'a> {
    fn new(bytes: &'a [u8]) -> Self { Self { bytes, pos: 0 } }

    fn u8(&mut self) -> Result<u8, IrError> {
        let b = self.bytes.get(self.pos).copied().ok_or_else(|| {
            IrError::Invalid(format!("BorderFill: u8 read OOB at {}", self.pos))
        })?;
        self.pos += 1;
        Ok(b)
    }

    fn u16(&mut self) -> Result<u16, IrError> {
        self.chunk::<2>().map(u16::from_le_bytes)
    }

    fn u32(&mut self) -> Result<u32, IrError> {
        self.chunk::<4>().map(u32::from_le_bytes)
    }

    fn chunk<const N: usize>(&mut self) -> Result<[u8; N], IrError> {
        if self.pos + N > self.bytes.len() {
            return Err(IrError::Invalid(format!(
                "BorderFill: {N}-byte read OOB at {}/{}",
                self.pos,
                self.bytes.len()
            )));
        }
        let mut buf = [0u8; N];
        buf.copy_from_slice(&self.bytes[self.pos..self.pos + N]);
        self.pos += N;
        Ok(buf)
    }

    fn border(&mut self) -> Result<Border, IrError> {
        let kind = self.u8()?;
        let width = self.u8()?;
        let color = self.u32()?;
        Ok(Border { kind, width, color })
    }

    fn remaining(&self) -> &[u8] {
        &self.bytes[self.pos..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(data: Vec<u8>) -> Record {
        Record { tag: tag::BORDER_FILL, level: 1, data }
    }

    #[test]
    fn default_roundtrips() {
        let bf = BorderFill::default();
        let bytes = emit(&bf);
        assert_eq!(bytes.len(), HEADER_SIZE);
        assert_eq!(parse(&rec(bytes)).unwrap(), bf);
    }

    #[test]
    fn with_color_fill() {
        let bf = BorderFill {
            attribute: 0x0041,
            borders: [
                Border { kind: 1, width: 2, color: 0xFF_00_00_00 },
                Border { kind: 1, width: 2, color: 0xFF_00_00_00 },
                Border { kind: 3, width: 5, color: 0xFF_AA_BB_CC },
                Border { kind: 3, width: 5, color: 0xFF_AA_BB_CC },
            ],
            diagonal: Border { kind: 0, width: 0, color: 0 },
            fill: Fill {
                kind: Fill::KIND_COLOR,
                body: b"opaque fill body bytes".to_vec(),
            },
        };
        let bytes = emit(&bf);
        let parsed = parse(&rec(bytes)).unwrap();
        assert_eq!(parsed, bf);
        assert!(parsed.fill.is_color());
        assert!(!parsed.fill.is_gradation());
    }

    #[test]
    fn wrong_tag_errors() {
        let r = Record { tag: tag::FACE_NAME, level: 0, data: vec![0; HEADER_SIZE] };
        assert!(parse(&r).is_err());
    }

    #[test]
    fn short_data_errors() {
        let r = rec(vec![0; HEADER_SIZE - 1]);
        assert!(parse(&r).is_err());
    }

    #[test]
    fn empty_body_ok() {
        let r = rec(vec![0; HEADER_SIZE]);
        let parsed = parse(&r).unwrap();
        assert!(parsed.fill.body.is_empty());
    }
}
