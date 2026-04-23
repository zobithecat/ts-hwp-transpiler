//! Minimum-viable HWPX reader.
//!
//! Pulls `Contents/section{N}.xml` out of the OCF (ZIP) container,
//! streams each XML file into an IR `Section`, and stacks them
//! into a populated `IrDocument`. Other parts — `Contents/header.xml`
//! for styles/fonts, `BinData/*` for images — are surfaced through
//! `IrDocument::unknown_streams` so a future writer round-trips them
//! verbatim, but they're not decoded in this pass.

use hwp_transpiler_core::ir::{IrDocument, IrError, Reader};

use super::header_xml::parse_header_xml;
use super::section_xml::parse_section_xml;
use super::zip_reader::HwpxArchive;

#[derive(Default)]
pub struct HwpxReader;

impl Reader for HwpxReader {
    fn read(&mut self, input: &[u8]) -> Result<IrDocument, IrError> {
        let mut archive = HwpxArchive::new(input.to_vec())?;

        let mut doc = IrDocument::default();

        // header.xml → DocInfo-ish records (font faces, border fills,
        // char shapes). Missing or malformed is non-fatal — the
        // section parse below still produces a usable document, it
        // just lacks styling context for the role classifier / inline
        // formatting path.
        if let Some(header_bytes) = archive.try_read_part("Contents/header.xml")? {
            if let Ok(hdr) = parse_header_xml(&header_bytes) {
                doc.doc_info.font_faces = hdr.font_faces;
                doc.doc_info.border_fills = hdr.border_fills;
                doc.doc_info.char_shapes = hdr.char_shapes;
            }
        }

        // Collect `Contents/section*.xml` names up front, then sort so
        // section0 / section1 / ... are read in index order regardless
        // of how the ZIP's central directory stored them.
        let mut section_names: Vec<String> = archive
            .entry_names()
            .into_iter()
            .filter(|n| is_section_xml(n))
            .collect();
        section_names.sort_by_key(section_index);

        for name in &section_names {
            let bytes = archive.read_part(name)?;
            let section = parse_section_xml(&bytes)?;
            doc.sections.push(section);
        }

        // Every other part goes into `unknown_streams` keyed by its
        // archive path. Callers that want images or styles can peek
        // there; a later typed decoder can promote them out.
        for name in archive.entry_names() {
            if is_section_xml(&name) {
                continue;
            }
            // Skip the OCF signature and zero-size entries — they're
            // noise for a reader that only surfaces real payloads.
            if name == "mimetype" {
                continue;
            }
            if let Some(bytes) = archive.try_read_part(&name)? {
                if bytes.is_empty() {
                    continue;
                }
                doc.unknown_streams.insert(name, bytes);
            }
        }

        Ok(doc)
    }
}

fn is_section_xml(name: &str) -> bool {
    // `Contents/section0.xml`, `Contents/section1.xml`, ...
    let Some(rest) = name.strip_prefix("Contents/section") else {
        return false;
    };
    let Some(idx) = rest.strip_suffix(".xml") else {
        return false;
    };
    !idx.is_empty() && idx.chars().all(|c| c.is_ascii_digit())
}

fn section_index(name: &String) -> usize {
    name.strip_prefix("Contents/section")
        .and_then(|s| s.strip_suffix(".xml"))
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_section_paths() {
        assert!(is_section_xml("Contents/section0.xml"));
        assert!(is_section_xml("Contents/section12.xml"));
        assert!(!is_section_xml("Contents/section.xml"));
        assert!(!is_section_xml("Contents/sectionAbc.xml"));
        assert!(!is_section_xml("Contents/header.xml"));
    }

    #[test]
    fn section_index_orders_by_number() {
        assert_eq!(section_index(&"Contents/section0.xml".into()), 0);
        assert_eq!(section_index(&"Contents/section10.xml".into()), 10);
    }
}
