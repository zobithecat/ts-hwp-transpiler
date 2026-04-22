//! PARA_LINE_SEG (tag 0x0045) — list of `LineSegment` entries.
//!
//! Each entry = 36 bytes = 9 × 32-bit fields:
//!   u32 text_start
//!   i32 vertical_position
//!   i32 line_height
//!   i32 text_height
//!   i32 baseline_distance
//!   i32 line_spacing
//!   i32 start_x
//!   i32 width
//!   u32 tag (flags)

use hwp_transpiler_core::ir::{IrError, LineSegment, Record};

use super::body_text::tag;

const ENTRY: usize = 36;

pub fn parse(rec: &Record) -> Result<Vec<LineSegment>, IrError> {
    if rec.tag != tag::PARA_LINE_SEG {
        return Err(IrError::Invalid(format!(
            "expected PARA_LINE_SEG (0x{:04X}), got 0x{:04X}",
            tag::PARA_LINE_SEG,
            rec.tag
        )));
    }
    if rec.data.len() % ENTRY != 0 {
        return Err(IrError::Invalid(format!(
            "PARA_LINE_SEG not {ENTRY}B-aligned: {} bytes",
            rec.data.len()
        )));
    }
    Ok(rec
        .data
        .chunks_exact(ENTRY)
        .map(|c| {
            let u32_at = |off: usize| {
                u32::from_le_bytes([c[off], c[off + 1], c[off + 2], c[off + 3]])
            };
            let i32_at = |off: usize| u32_at(off) as i32;
            LineSegment {
                text_start: u32_at(0),
                vertical_position_hwpu: i32_at(4),
                line_height_hwpu: i32_at(8),
                text_height_hwpu: i32_at(12),
                baseline_distance_hwpu: i32_at(16),
                line_spacing_hwpu: i32_at(20),
                start_x_hwpu: i32_at(24),
                width_hwpu: i32_at(28),
                tag: u32_at(32),
            }
        })
        .collect())
}

pub fn emit(segs: &[LineSegment]) -> Vec<u8> {
    let mut out = Vec::with_capacity(segs.len() * ENTRY);
    for s in segs {
        out.extend_from_slice(&s.text_start.to_le_bytes());
        out.extend_from_slice(&s.vertical_position_hwpu.to_le_bytes());
        out.extend_from_slice(&s.line_height_hwpu.to_le_bytes());
        out.extend_from_slice(&s.text_height_hwpu.to_le_bytes());
        out.extend_from_slice(&s.baseline_distance_hwpu.to_le_bytes());
        out.extend_from_slice(&s.line_spacing_hwpu.to_le_bytes());
        out.extend_from_slice(&s.start_x_hwpu.to_le_bytes());
        out.extend_from_slice(&s.width_hwpu.to_le_bytes());
        out.extend_from_slice(&s.tag.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(data: Vec<u8>) -> Record {
        Record { tag: tag::PARA_LINE_SEG, level: 1, data }
    }

    #[test]
    fn roundtrip() {
        let segs = vec![
            LineSegment {
                text_start: 0,
                vertical_position_hwpu: 0,
                line_height_hwpu: 1600,
                text_height_hwpu: 1000,
                baseline_distance_hwpu: 800,
                line_spacing_hwpu: 600,
                start_x_hwpu: 0,
                width_hwpu: 30000,
                tag: 0x01,
            },
            LineSegment {
                text_start: 25,
                vertical_position_hwpu: 1600,
                line_height_hwpu: 1600,
                text_height_hwpu: 1000,
                baseline_distance_hwpu: 800,
                line_spacing_hwpu: 600,
                start_x_hwpu: 0,
                width_hwpu: 30000,
                tag: 0x00,
            },
        ];
        let bytes = emit(&segs);
        assert_eq!(bytes.len(), 2 * ENTRY);
        assert_eq!(parse(&rec(bytes)).unwrap(), segs);
    }

    #[test]
    fn empty_ok() {
        assert_eq!(parse(&rec(vec![])).unwrap(), Vec::<LineSegment>::new());
    }

    #[test]
    fn misaligned_errors() {
        assert!(parse(&rec(vec![0; 35])).is_err());
    }

    #[test]
    fn wrong_tag_errors() {
        let r = Record { tag: tag::PARA_HEADER, level: 0, data: vec![] };
        assert!(parse(&r).is_err());
    }
}
