//! HWPX container writer.
//!
//! Strategy — same "verbatim cache for parts not yet typed" shape as
//! the HWP5 writer: sections round-trip through a fresh XML emit
//! (since the reader decoded them into typed IR); the original
//! `Contents/header.xml` flows through a *surgical rewriter* that
//! overlays only the attributes the IR exposes (paraPr align, charPr
//! parent attrs, font face names, borderFill solid colours, strike /
//! underline shape attrs); every other archive part rides along
//! verbatim from `doc.unknown_streams`. This lets a DocInfo-side
//! mutation flow into the written file without re-emitting from
//! scratch (which would lose the unparsed sections — styles,
//! numberings, lineSpacing, kerning flags, Panose typeInfo).
//!
//! Missing compared to a full writer:
//!   * Bold / italic toggling on individual char shapes — the HWPX
//!     `<hh:bold/>` / `<hh:italic/>` are presence-only and would
//!     require structural insert / remove inside `<hh:charPr>`. The
//!     unmutated round-trip path works (those bytes flow verbatim).
//!   * Multi-script CharShape arrays (`<hh:fontRef>`, `<hh:ratio>`,
//!     …) are not overlaid. Mutating them in IR will not flow into
//!     the written file.
//!   * Adding / removing whole shapes (new charPr, new paraPr) is
//!     structural and not supported.
//!   * Pictures / equations in paragraph controls don't emit back
//!     into the XML; they still exist in the IR and would need their
//!     own serialisers to reach the output.
//!   * New sections beyond what the reader saw aren't referenced from
//!     `Contents/content.hpf` (which stays verbatim); viewers may
//!     ignore the extras.

use hwp_transpiler_core::ir::{IrDocument, IrError, Writer};

use super::header_rewriter::rewrite_header_xml;
use super::section_writer::write_section_xml;
use super::zip_writer::HwpxArchiveWriter;

const MIMETYPE: &str = "application/hwp+zip";

#[derive(Default)]
pub struct HwpxWriter;

impl Writer for HwpxWriter {
    fn write(&mut self, doc: &IrDocument) -> Result<Vec<u8>, IrError> {
        let mut zip = HwpxArchiveWriter::new();

        // OCF requires mimetype first, stored uncompressed.
        zip.write_mimetype(MIMETYPE)?;

        // Regenerate sections from IR — they carry the mutations we
        // care about (typed paragraphs, tables, cell spans).
        for (i, section) in doc.sections.iter().enumerate() {
            let xml = write_section_xml(section)?;
            zip.add_part(&format!("Contents/section{i}.xml"), &xml)?;
        }

        // Re-emit embedded binaries under `BinData/<id>` — the reader
        // promotes those out of `unknown_streams` into `doc.bin_data`
        // so the HTML preview can resolve `binaryItemIDRef`; the
        // writer has to put them back.
        for entry in &doc.bin_data {
            if entry.bytes.is_empty() {
                continue;
            }
            zip.add_part(&format!("BinData/{}", entry.id), &entry.bytes)?;
        }

        // Passthrough every other archive part verbatim. Section
        // entries from the original are skipped because we just
        // emitted fresh ones — if an IR-side edit changed the
        // section count the original entries would conflict otherwise.
        // `mimetype` is also skipped because `write_mimetype` already
        // placed it, stored uncompressed. `BinData/*` is skipped
        // because bin_data above already handled it.
        for (name, bytes) in &doc.unknown_streams {
            if name == "mimetype" {
                continue;
            }
            if is_section_xml(name) {
                continue;
            }
            if name.starts_with("BinData/") {
                continue;
            }
            if name == "Contents/header.xml" {
                // Overlay IR-side DocInfo mutations on top of the
                // original header.xml bytes. Falls back to the
                // verbatim original if the rewriter chokes on the
                // input — better to ship an unmutated header than a
                // corrupted one.
                let rewritten = rewrite_header_xml(bytes, doc).unwrap_or_else(|_| bytes.clone());
                zip.add_part(name, &rewritten)?;
                continue;
            }
            zip.add_part(name, bytes)?;
        }

        zip.finish()
    }
}

fn is_section_xml(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("Contents/section") else {
        return false;
    };
    let Some(idx) = rest.strip_suffix(".xml") else {
        return false;
    };
    !idx.is_empty() && idx.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hwp_transpiler_core::ir::{Paragraph, Reader, Section};

    fn doc_with_one_paragraph(text: &str) -> IrDocument {
        let mut doc = IrDocument::default();
        let mut section = Section::default();
        section.paragraphs.push(Paragraph {
            text: text.into(),
            ..Paragraph::default()
        });
        doc.sections.push(section);
        doc
    }

    #[test]
    fn emits_valid_zip_with_pk_magic() {
        let doc = doc_with_one_paragraph("hi");
        let bytes = HwpxWriter::default().write(&doc).expect("write");
        assert_eq!(&bytes[0..4], b"PK\x03\x04");
    }

    #[test]
    fn mimetype_is_first_entry() {
        let doc = doc_with_one_paragraph("hi");
        let bytes = HwpxWriter::default().write(&doc).expect("write");
        let off = 30 + "mimetype".len();
        assert_eq!(&bytes[off..off + MIMETYPE.len()], MIMETYPE.as_bytes());
    }

    #[test]
    fn round_trip_through_reader_preserves_paragraph_text() {
        let doc = doc_with_one_paragraph("round trip");
        let bytes = HwpxWriter::default().write(&doc).expect("write");
        // Re-read the emitted archive to confirm our section XML is
        // structurally valid and our reader agrees.
        let mut r = super::super::reader::HwpxReader;
        let read = r.read(&bytes).expect("read");
        assert_eq!(read.sections.len(), 1);
        assert_eq!(read.sections[0].paragraphs[0].text, "round trip");
    }

    #[test]
    fn is_section_xml_matches_expected_paths() {
        assert!(is_section_xml("Contents/section0.xml"));
        assert!(is_section_xml("Contents/section9.xml"));
        assert!(!is_section_xml("Contents/section.xml"));
        assert!(!is_section_xml("Contents/header.xml"));
        assert!(!is_section_xml("BinData/image1.png"));
    }
}
