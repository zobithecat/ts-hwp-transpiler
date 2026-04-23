//! PAGE_DEF (tag 0x0049) record decoder.
//!
//! Layout (hwplib `PageDefReader`, 10 × u32 = 40 bytes):
//!
//!   u32   paper_width       (HWPUNIT)
//!   u32   paper_height      (HWPUNIT)
//!   u32   left_margin
//!   u32   right_margin
//!   u32   top_margin
//!   u32   bottom_margin
//!   u32   header_margin
//!   u32   footer_margin
//!   u32   gutter_margin
//!   u32   properties         (landscape / gutter_kind / paper_kind bits)
//!
//! The decoder populates [`SectionProperties`] with page size and the
//! four core margins (reordered to `[top, right, bottom, left]` to
//! match the IR convention). Header/footer/gutter and the `properties`
//! bitfield are *not* surfaced yet — the write path keeps the PAGE_DEF
//! record verbatim in `section.raw_records`, so losing them in the
//! typed view is fine for now.

use hwp_transpiler_core::ir::{IrError, Record, SectionProperties};

use super::body_text::tag;

const EXPECTED_LEN: usize = 10 * 4;

pub fn parse(rec: &Record) -> Result<SectionProperties, IrError> {
    if rec.tag != tag::PAGE_DEF {
        return Err(IrError::Invalid(format!(
            "expected PAGE_DEF (0x{:04X}), got 0x{:04X}",
            tag::PAGE_DEF,
            rec.tag
        )));
    }
    if rec.data.len() < EXPECTED_LEN {
        return Err(IrError::Invalid(format!(
            "PAGE_DEF too short: {} < {EXPECTED_LEN}",
            rec.data.len()
        )));
    }

    let paper_width = u32_at(&rec.data, 0);
    let paper_height = u32_at(&rec.data, 4);
    let left = u32_at(&rec.data, 8);
    let right = u32_at(&rec.data, 12);
    let top = u32_at(&rec.data, 16);
    let bottom = u32_at(&rec.data, 20);
    // bytes 24..=39 hold header/footer/gutter/properties — kept in the
    // verbatim raw_record for round-trip; not surfaced here.

    Ok(SectionProperties {
        page_width_hwpu: paper_width,
        page_height_hwpu: paper_height,
        margins_hwpu: [top, right, bottom, left],
        // columns live in a separate record (COLUMN_DEFINITION / SHAPE_
        // COMPONENT variants depending on the doc). Leave default until
        // that decoder lands.
        columns: 0,
    })
}

fn u32_at(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_record(bytes: Vec<u8>) -> Record {
        Record {
            tag: tag::PAGE_DEF,
            level: 0,
            data: bytes,
        }
    }

    #[test]
    fn decodes_a4_portrait() {
        // A4 portrait: ~210mm × 297mm → 59528 × 84168 HWPUNIT.
        let mut bytes = Vec::with_capacity(EXPECTED_LEN);
        for v in [
            59528u32, 84168, 4251, 4251, 5669, 5669, 4251, 4251, 0, 0,
        ] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let props = parse(&mk_record(bytes)).expect("parse ok");
        assert_eq!(props.page_width_hwpu, 59528);
        assert_eq!(props.page_height_hwpu, 84168);
        // Reordered left/right/top/bottom → top/right/bottom/left.
        assert_eq!(props.margins_hwpu, [5669, 4251, 5669, 4251]);
    }

    #[test]
    fn rejects_wrong_tag() {
        let rec = Record {
            tag: 0x0001,
            level: 0,
            data: vec![0; EXPECTED_LEN],
        };
        assert!(parse(&rec).is_err());
    }

    #[test]
    fn rejects_truncated() {
        let rec = mk_record(vec![0; EXPECTED_LEN - 1]);
        assert!(parse(&rec).is_err());
    }
}
