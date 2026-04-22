//! IdMappings (tag 0x0011) — flat array of u32 LE counts. Number of entries
//! depends on HWP5 revision (15 through 18 at the time of writing).
//!
//! | idx | meaning                    | since    |
//! |-----|----------------------------|----------|
//! |  0  | BinData                    | 5.0.0.0  |
//! |  1  | Hangul font slot count     | 5.0.0.0  |
//! |  2  | Latin font slot count      | 5.0.0.0  |
//! |  3  | Hanja font slot count      | 5.0.0.0  |
//! |  4  | Japanese font slot count   | 5.0.0.0  |
//! |  5  | Other font slot count      | 5.0.0.0  |
//! |  6  | Symbol font slot count     | 5.0.0.0  |
//! |  7  | User font slot count       | 5.0.0.0  |
//! |  8  | BorderFill                 | 5.0.0.0  |
//! |  9  | CharShape                  | 5.0.0.0  |
//! | 10  | TabDef                     | 5.0.0.0  |
//! | 11  | Numbering                  | 5.0.0.0  |
//! | 12  | Bullet                     | 5.0.0.0  |
//! | 13  | ParaShape                  | 5.0.0.0  |
//! | 14  | Style                      | 5.0.0.0  |
//! | 15  | MemoShape                  | 5.0.1.6  |
//! | 16  | TrackChange                | 5.0.3.0  |
//! | 17  | TrackChangeAuthor          | 5.0.3.0  |

use hwp_transpiler_core::ir::{IrError, Record};

use super::doc_info::tag;

pub const MIN_FIELDS: usize = 15;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdMappings {
    pub bin_data: u32,
    /// [Hangul, Latin, Hanja, Japanese, Other, Symbol, User]
    pub fonts: [u32; 7],
    pub border_fill: u32,
    pub char_shape: u32,
    pub tab_def: u32,
    pub numbering: u32,
    pub bullet: u32,
    pub para_shape: u32,
    pub style: u32,
    /// MemoShape, TrackChange, TrackChangeAuthor, future additions.
    pub extra: Vec<u32>,
}

impl IdMappings {
    pub fn total_font_count(&self) -> u32 {
        self.fonts.iter().sum()
    }
}

pub fn parse(rec: &Record) -> Result<IdMappings, IrError> {
    if rec.tag != tag::ID_MAPPINGS {
        return Err(IrError::Invalid(format!(
            "expected IdMappings (0x{:04X}), got 0x{:04X}",
            tag::ID_MAPPINGS,
            rec.tag
        )));
    }
    if rec.data.len() % 4 != 0 {
        return Err(IrError::Invalid(format!(
            "IdMappings not u32-aligned: {} bytes",
            rec.data.len()
        )));
    }
    let n = rec.data.len() / 4;
    if n < MIN_FIELDS {
        return Err(IrError::Invalid(format!(
            "IdMappings has {} fields, min {}",
            n,
            MIN_FIELDS
        )));
    }
    let u32s: Vec<u32> = rec
        .data
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    Ok(IdMappings {
        bin_data: u32s[0],
        fonts: [u32s[1], u32s[2], u32s[3], u32s[4], u32s[5], u32s[6], u32s[7]],
        border_fill: u32s[8],
        char_shape: u32s[9],
        tab_def: u32s[10],
        numbering: u32s[11],
        bullet: u32s[12],
        para_shape: u32s[13],
        style: u32s[14],
        extra: u32s[15..].to_vec(),
    })
}

pub fn emit(m: &IdMappings) -> Vec<u8> {
    let mut out = Vec::with_capacity((MIN_FIELDS + m.extra.len()) * 4);
    let mut push = |v: u32| out.extend_from_slice(&v.to_le_bytes());
    push(m.bin_data);
    for f in m.fonts {
        push(f);
    }
    push(m.border_fill);
    push(m.char_shape);
    push(m.tab_def);
    push(m.numbering);
    push(m.bullet);
    push(m.para_shape);
    push(m.style);
    for &e in &m.extra {
        push(e);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> IdMappings {
        IdMappings {
            bin_data: 0,
            fonts: [2, 2, 2, 2, 2, 2, 2],
            border_fill: 3,
            char_shape: 5,
            tab_def: 1,
            numbering: 1,
            bullet: 0,
            para_shape: 8,
            style: 20,
            extra: vec![1, 0, 0],
        }
    }

    #[test]
    fn roundtrip() {
        let m = sample();
        let bytes = emit(&m);
        let rec = Record { tag: tag::ID_MAPPINGS, level: 0, data: bytes };
        assert_eq!(parse(&rec).unwrap(), m);
    }

    #[test]
    fn total_font_count() {
        assert_eq!(sample().total_font_count(), 14);
    }

    #[test]
    fn wrong_tag_errors() {
        let rec = Record { tag: 0x10, level: 0, data: vec![0; 60] };
        assert!(parse(&rec).is_err());
    }

    #[test]
    fn short_data_errors() {
        let rec = Record { tag: tag::ID_MAPPINGS, level: 0, data: vec![0; 4 * (MIN_FIELDS - 1)] };
        assert!(parse(&rec).is_err());
    }

    #[test]
    fn unaligned_data_errors() {
        let rec = Record { tag: tag::ID_MAPPINGS, level: 0, data: vec![0; 4 * MIN_FIELDS + 1] };
        assert!(parse(&rec).is_err());
    }
}
