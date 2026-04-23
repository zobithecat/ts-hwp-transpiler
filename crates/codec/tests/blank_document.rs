//! `blank_document()` seeds a minimal valid HWP — it's the starting
//! point for any caller that wants to build an HWP file from code
//! (MD → HWP, CLI scaffolds, tests) without hunting for a template
//! file on disk. These tests check that:
//!
//!   1. Parsing succeeds.
//!   2. The structure matches what our probe saw in hwplib's
//!      `BlankFileMaker` output (section count, paragraph count,
//!      DocInfo record count, at least one paragraph, …).
//!   3. `HwpWriter.write` accepts the IR and the written bytes
//!      re-parse into an equivalent document.
//!   4. Callers can mutate the seed (add a paragraph, edit a header)
//!      and the mutation flows through the write path — proves the
//!      seed is actually usable as a starting point, not just an
//!      opaque verbatim blob.

use hwp_transpiler_codec::hwp::{HwpReader, HwpWriter, blank_document};
use hwp_transpiler_core::ir::{Reader, Writer};

#[test]
fn blank_document_parses() {
    let doc = blank_document().expect("blank_document");
    assert!(
        !doc.sections.is_empty(),
        "blank template must contain at least one section"
    );
    assert!(
        !doc.sections[0].paragraphs.is_empty(),
        "section 0 must contain at least one paragraph"
    );
    assert!(
        !doc.doc_info.raw_records.is_empty(),
        "DocInfo must carry records"
    );
}

#[test]
fn blank_document_matches_fixture_shape() {
    // Anchors taken from the probe dump of hwplib's blank.hwp. If
    // we ever swap the embedded template, these should be updated
    // intentionally, not drift silently.
    let doc = blank_document().expect("blank_document");
    assert_eq!(doc.sections.len(), 1);
    assert_eq!(doc.sections[0].paragraphs.len(), 1);
    assert_eq!(doc.doc_info.raw_records.len(), 56);
    assert_eq!(doc.doc_info.border_fills.len(), 2);
    assert_eq!(doc.doc_info.char_shapes.len(), 5);
    assert_eq!(doc.doc_info.para_shapes.len(), 12);
    assert_eq!(doc.doc_info.styles.len(), 14);
    assert_eq!(doc.doc_info.font_faces.hangul.len(), 2);
    assert_eq!(doc.doc_info.font_faces.latin.len(), 2);
    assert_eq!(doc.doc_info.font_faces.hanja.len(), 2);
    assert_eq!(doc.bin_data.len(), 1);
}

#[test]
fn blank_document_writes_and_reparses() {
    let doc = blank_document().expect("blank_document");
    let out = HwpWriter.write(&doc).expect("write blank");
    let after = HwpReader.read(&out).expect("re-read blank");

    assert_eq!(after.sections.len(), doc.sections.len());
    assert_eq!(
        after.sections[0].paragraphs.len(),
        doc.sections[0].paragraphs.len()
    );
    assert_eq!(
        after.doc_info.raw_records.len(),
        doc.doc_info.raw_records.len()
    );
    // DocInfo record tag sequence should be preserved.
    let before_tags: Vec<u16> = doc.doc_info.raw_records.iter().map(|r| r.tag).collect();
    let after_tags: Vec<u16> = after.doc_info.raw_records.iter().map(|r| r.tag).collect();
    assert_eq!(before_tags, after_tags);
}

#[test]
fn blank_document_seed_accepts_mutation() {
    // Prove the seed isn't just an opaque verbatim passthrough — a
    // typed edit (style_id on paragraph 0) must survive a write /
    // read cycle, which forces the re-encode path that Phase 1 wired.
    let mut doc = blank_document().expect("blank_document");

    let original = doc.sections[0].paragraphs[0].header.style_id;
    let mutated = original.wrapping_add(1);
    doc.sections[0].paragraphs_mut()[0].header.style_id = mutated;

    let out = HwpWriter.write(&doc).expect("write mutated blank");
    let after = HwpReader.read(&out).expect("re-read mutated blank");

    assert_eq!(
        after.sections[0].paragraphs[0].header.style_id,
        mutated,
        "blank_document seed should carry user mutations through the writer"
    );
}
