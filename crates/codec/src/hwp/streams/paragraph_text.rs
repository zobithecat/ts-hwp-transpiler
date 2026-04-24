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
                    // Graceful stop rather than hard error. Real-world
                    // HWP5 documents (observed on a 62 MB report
                    // fixture) sometimes end PARA_TEXT with a stray
                    // low-value unit that doesn't leave room for the
                    // 8-unit extended payload — padding artefacts
                    // from the authoring tool. Surface what we got so
                    // far; the caller still gets the record's typed
                    // view and the raw bytes stay in `raw_records`
                    // for byte-equal round-trip.
                    break;
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

/// Inverse of [`extract_text`]: serialize `text` back into a raw
/// PARA_TEXT payload (UTF-16LE with the HWP5 inline-control reverse
/// mapping applied).
///
/// Refuses to emit when the input contains [`OBJECT_REPLACEMENT`]
/// (U+FFFC) — that placeholder means the typed `text` view is incomplete
/// because the original record had extended (16-byte) controls like
/// inline pictures or field markers. Re-emitting from `text` alone
/// would destroy those control payloads; callers that hit this error
/// should keep the paragraph's verbatim PARA_TEXT record instead.
///
/// Also refuses to emit when `text` contains bare U+0000..=U+001F code
/// points outside the inline-control whitelist (TAB / LF / NBSP /
/// SOFT HYPHEN / EM SPACE). Such code points would be decoded as HWP5
/// extended control markers and corrupt the record on parse.
pub fn emit(text: &str) -> Result<Vec<u8>, IrError> {
    if text.contains('\u{FFFC}') {
        return Err(IrError::Unsupported(
            "PARA_TEXT: cannot re-encode text containing U+FFFC \
             (extended-control placeholder); keep verbatim raw_record"
                .into(),
        ));
    }
    let mut out = Vec::with_capacity(text.len() * 2);
    for c in text.chars() {
        let mut buf = [0u16; 2];
        let units = c.encode_utf16(&mut buf);
        for u in units.iter().copied() {
            let emitted: u16 = match u {
                // Inline controls that pass through (stay as HWP5 codes).
                0x0009 | 0x000A => u,
                // Unicode → HWP5 inline reverse map.
                0x00A0 => 0x001E, // NBSP
                0x00AD => 0x0018, // SOFT HYPHEN
                0x2003 => 0x001F, // EM SPACE
                // Any other low-value code point would be read as an
                // extended-control marker. Refuse rather than silently
                // corrupting the stream.
                0x0000..=0x001F => {
                    return Err(IrError::Unsupported(format!(
                        "PARA_TEXT: cannot emit bare low-value char \
                         U+{u:04X} — would decode as an HWP5 extended \
                         control on read"
                    )));
                }
                _ => u,
            };
            out.extend_from_slice(&emitted.to_le_bytes());
        }
    }
    Ok(out)
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
    fn truncated_extended_stops_gracefully() {
        // A truncated extended control at the end of PARA_TEXT used
        // to error; we now return what we parsed so far so large
        // real-world documents (observed on a 62 MB report fixture
        // with a trailing null-unit artefact) still load.
        let mut data = utf16le("a");
        data.extend_from_slice(&[0x0B, 0, 0, 0]); // code + only 1 data u16
        assert_eq!(extract_text(&rec(data)).unwrap(), "a");
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

    #[test]
    fn emit_plain_ascii_roundtrips() {
        let bytes = emit("hello").unwrap();
        assert_eq!(bytes, utf16le("hello"));
        assert_eq!(extract_text(&rec(bytes)).unwrap(), "hello");
    }

    #[test]
    fn emit_hangul_roundtrips() {
        let bytes = emit("안녕하세요").unwrap();
        assert_eq!(bytes, utf16le("안녕하세요"));
        assert_eq!(extract_text(&rec(bytes)).unwrap(), "안녕하세요");
    }

    #[test]
    fn emit_tab_preserved_inline() {
        let bytes = emit("a\tb").unwrap();
        assert_eq!(bytes, vec![b'a', 0, 0x09, 0, b'b', 0]);
    }

    #[test]
    fn emit_nbsp_reverses_to_hwp5_control() {
        let bytes = emit("a\u{00A0}b").unwrap();
        assert_eq!(bytes, vec![b'a', 0, 0x1E, 0, b'b', 0]);
        // Round-trip through extract_text must land back at U+00A0.
        assert_eq!(extract_text(&rec(bytes)).unwrap(), "a\u{00A0}b");
    }

    #[test]
    fn emit_em_space_reverses() {
        let bytes = emit("\u{2003}").unwrap();
        assert_eq!(bytes, vec![0x1F, 0]);
    }

    #[test]
    fn emit_soft_hyphen_reverses() {
        let bytes = emit("\u{00AD}").unwrap();
        assert_eq!(bytes, vec![0x18, 0]);
    }

    #[test]
    fn emit_rejects_object_replacement() {
        // U+FFFC means "extended control elsewhere in the raw record";
        // cannot round-trip through the typed view alone.
        assert!(emit("a\u{FFFC}b").is_err());
    }

    #[test]
    fn emit_rejects_bare_control_char() {
        // U+0005 has no inline mapping and would be misread as an
        // extended control marker.
        assert!(emit("a\u{0005}b").is_err());
    }

    #[test]
    fn emit_handles_surrogate_pairs() {
        // U+1F600 (🙀-ish emoji range) encodes as a UTF-16 surrogate
        // pair. Both halves must be emitted without either being
        // mistaken for a low-value control.
        let s = "x\u{1F600}y";
        let bytes = emit(s).unwrap();
        assert_eq!(bytes, utf16le(s));
        assert_eq!(extract_text(&rec(bytes)).unwrap(), s);
    }

    #[test]
    fn emit_extract_roundtrip_on_fixture_bytes() {
        // Synthesise a pure-text PARA_TEXT payload (no extended
        // controls), parse it, re-emit it, and expect byte equality.
        // This is the invariant sync_paragraph_records relies on for
        // unmutated paragraphs.
        let original = {
            let mut v = Vec::new();
            v.extend_from_slice(&utf16le("안"));
            v.extend_from_slice(&[0x09, 0]); // tab
            v.extend_from_slice(&utf16le("녕"));
            v.extend_from_slice(&[0x1E, 0]); // NBSP
            v.extend_from_slice(&utf16le("하"));
            v
        };
        let text = extract_text(&rec(original.clone())).unwrap();
        let reemit = emit(&text).unwrap();
        assert_eq!(reemit, original);
    }
}
