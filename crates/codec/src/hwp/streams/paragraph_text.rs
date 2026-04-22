//! PARA_TEXT (tag 0x0051) — UTF-16LE paragraph text.
//!
//! HWP control characters are code units < 0x20. Most are *extended*: one
//! 16-byte block = 8 UTF-16 units (control code + 6 data units + trailing
//! control code). A few are *inline*: a single UTF-16 unit that maps to a
//! concrete Unicode char.
//!
//! Inline (kept verbatim-ish):
//!   0x09  → U+0009 (TAB)
//!   0x0A  → U+000A (LINE FEED, soft line break)
//!   0x0D  → skipped (paragraph break; shouldn't appear mid-record)
//!   0x18  → U+00AD (SOFT HYPHEN / auto-hyphen)
//!   0x1E  → U+00A0 (NON-BREAKING SPACE)
//!   0x1F  → U+2003 (EM SPACE / fixed-width space)
//!
//! Extended (stripped to U+FFFC placeholder in the `text` view; opaque
//! payload stays in the paragraph's `raw_records` PARA_TEXT entry):
//!   0x00..=0x08, 0x0B, 0x0C, 0x0E..=0x17, 0x19..=0x1D

use hwp_transpiler_core::ir::{IrError, Record};

use super::body_text::tag;

pub const OBJECT_REPLACEMENT: u16 = 0xFFFC;
const EXTENDED_UNITS: usize = 8;

pub fn extract_text(rec: &Record) -> Result<String, IrError> {
    if rec.tag != tag::PARA_TEXT {
        return Err(IrError::Invalid(format!(
            "expected PARA_TEXT (0x{:04X}), got 0x{:04X}",
            tag::PARA_TEXT,
            rec.tag
        )));
    }
    if rec.data.len() % 2 != 0 {
        return Err(IrError::Invalid(format!(
            "PARA_TEXT not u16-aligned: {} bytes",
            rec.data.len()
        )));
    }
    let units: Vec<u16> = rec
        .data
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();

    let mut out: Vec<u16> = Vec::with_capacity(units.len());
    let mut i = 0;
    while i < units.len() {
        let u = units[i];
        match u {
            0x09 | 0x0A => {
                out.push(u);
                i += 1;
            }
            0x0D => {
                // Paragraph break should not appear inside PARA_TEXT; skip
                // defensively instead of erroring.
                i += 1;
            }
            0x18 => { out.push(0x00AD); i += 1; }
            0x1E => { out.push(0x00A0); i += 1; }
            0x1F => { out.push(0x2003); i += 1; }
            0x00..=0x1F => {
                if i + EXTENDED_UNITS > units.len() {
                    return Err(IrError::Invalid(format!(
                        "extended control 0x{:02X} truncated at unit {}",
                        u, i
                    )));
                }
                out.push(OBJECT_REPLACEMENT);
                i += EXTENDED_UNITS;
            }
            _ => {
                out.push(u);
                i += 1;
            }
        }
    }
    String::from_utf16(&out)
        .map_err(|e| IrError::Invalid(format!("PARA_TEXT UTF-16: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(data: Vec<u8>) -> Record {
        Record { tag: tag::PARA_TEXT, level: 1, data }
    }

    fn utf16le(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
    }

    #[test]
    fn plain_ascii() {
        assert_eq!(extract_text(&rec(utf16le("hello"))).unwrap(), "hello");
    }

    #[test]
    fn plain_hangul() {
        assert_eq!(extract_text(&rec(utf16le("안녕하세요"))).unwrap(), "안녕하세요");
    }

    #[test]
    fn inline_tab_preserved() {
        let data = [b'a', 0, 0x09, 0, b'b', 0];
        assert_eq!(extract_text(&rec(data.to_vec())).unwrap(), "a\tb");
    }

    #[test]
    fn nbsp_translated() {
        let data = [b'a', 0, 0x1E, 0, b'b', 0];
        assert_eq!(extract_text(&rec(data.to_vec())).unwrap(), "a\u{00A0}b");
    }

    #[test]
    fn extended_control_becomes_replacement_char() {
        // 'a' + 8 u16 extended control (code 0x0B, picture) + 'b'
        let mut data = utf16le("a");
        data.extend_from_slice(&[0x0B, 0]);
        data.extend_from_slice(&[0; 14]); // 7 more u16s
        data.extend_from_slice(&utf16le("b"));
        let text = extract_text(&rec(data)).unwrap();
        assert_eq!(text, "a\u{FFFC}b");
    }

    #[test]
    fn truncated_extended_errors() {
        let mut data = utf16le("a");
        data.extend_from_slice(&[0x0B, 0, 0, 0]); // code + only 1 data u16
        assert!(extract_text(&rec(data)).is_err());
    }

    #[test]
    fn odd_bytes_error() {
        assert!(extract_text(&rec(vec![0x41])).is_err());
    }

    #[test]
    fn empty_paragraph() {
        assert_eq!(extract_text(&rec(vec![])).unwrap(), "");
    }

    #[test]
    fn wrong_tag_errors() {
        let r = Record { tag: tag::PARA_HEADER, level: 0, data: vec![] };
        assert!(extract_text(&r).is_err());
    }
}
