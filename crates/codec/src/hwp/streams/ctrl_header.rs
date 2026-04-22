//! CTRL_HEADER (tag 0x0047) — marker record that identifies an inline
//! control object. First 4 bytes are the control ID (ASCII, little-endian
//! u32); subsequent bytes are type-specific fields consumed by the
//! follow-up records.
//!
//! Known control IDs (displayed in ASCII order):
//!
//! | id      | meaning                                             |
//! |---------|-----------------------------------------------------|
//! | "tbl "  | Table                                               |
//! | "pic "  | Picture                                             |
//! | "gso "  | General shape object                                |
//! | "eqed"  | Equation editor                                     |
//! | "tcps"  | Table cell page break settings                      |
//! | "sec "  | Section                                             |
//! | "cold"  | Column                                              |
//! | "fn  "  | Footnote                                            |
//! | "en  "  | Endnote                                             |
//! | "bokm"  | Bookmark                                            |
//! | "idxm"  | Index mark                                          |
//! | "atno"  | Autonum                                             |
//! | "head"  | Header                                              |
//! | "foot"  | Footer                                              |
//! | "hlnk"  | Hyperlink                                           |
//!
//! After Round 5c we only extract the id and the raw payload. Round 5d+
//! replaces the `Unknown` variant with typed `Table`/`Equation`/`Picture`.

use hwp_transpiler_core::ir::{Control, ControlKind, IrError, Record};

use super::body_text::tag;

pub fn parse(rec: &Record) -> Result<Control, IrError> {
    if rec.tag != tag::CTRL_HEADER {
        return Err(IrError::Invalid(format!(
            "expected CTRL_HEADER (0x{:04X}), got 0x{:04X}",
            tag::CTRL_HEADER,
            rec.tag
        )));
    }
    if rec.data.len() < 4 {
        return Err(IrError::Invalid(format!(
            "CTRL_HEADER too short: {} < 4",
            rec.data.len()
        )));
    }
    let code: [u8; 4] = rec.data[0..4].try_into().unwrap();
    let data = rec.data[4..].to_vec();
    Ok(Control {
        kind: ControlKind::Unknown { code, data },
    })
}

/// Render the 4-byte code as a readable string. HWP stores FOURCC-style
/// ids as little-endian u32, so file-order bytes are ASCII-reversed —
/// "tbl " ends up as `[' ', 'l', 'b', 't']` in [`ControlKind::Unknown::code`].
/// This function reverses back to the human-readable form. Bytes outside
/// printable ASCII are replaced with `.` for safe logging.
pub fn display_code(code: &[u8; 4]) -> String {
    code.iter()
        .rev()
        .map(|&b| if (0x20..=0x7E).contains(&b) { b as char } else { '.' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(data: Vec<u8>) -> Record {
        Record { tag: tag::CTRL_HEADER, level: 2, data }
    }

    #[test]
    fn parses_raw_bytes() {
        // File-order bytes for "tbl " end up reversed in `code`.
        let data = b" lbt\x01\x02\x03\x04".to_vec();
        let ctrl = parse(&rec(data)).unwrap();
        let ControlKind::Unknown { code, data } = &ctrl.kind else {
            panic!("expected Unknown variant");
        };
        assert_eq!(code, b" lbt");
        assert_eq!(display_code(code), "tbl ");
        assert_eq!(data, &[1, 2, 3, 4]);
    }

    #[test]
    fn displays_printable_code() {
        assert_eq!(display_code(b" lbt"), "tbl ");
        assert_eq!(display_code(b" cip"), "pic ");
        assert_eq!(display_code(&[0x7E, b'x', 0xFF, 0x01]), "..x~");
    }

    #[test]
    fn empty_payload_ok() {
        let ctrl = parse(&rec(b"deqe".to_vec())).unwrap();
        let ControlKind::Unknown { code, data } = &ctrl.kind else { panic!() };
        assert_eq!(display_code(code), "eqed");
        assert!(data.is_empty());
    }

    #[test]
    fn short_errors() {
        assert!(parse(&rec(vec![0; 3])).is_err());
    }

    #[test]
    fn wrong_tag_errors() {
        let r = Record { tag: tag::PARA_HEADER, level: 0, data: vec![0; 4] };
        assert!(parse(&r).is_err());
    }
}
