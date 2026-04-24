//! Integration smoke over a real HWPX fixture. Skips if the file
//! isn't on disk — the user-authored sample sits in `/test/` which
//! is git-ignored, so CI won't see it.

use hwp_transpiler_codec::hwpx::{HwpxReader, HwpxWriter};
use hwp_transpiler_core::ir::{ControlKind, Reader, Writer};
use std::path::Path;

fn repo_path(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(rel)
}

fn load_sample() -> Option<hwp_transpiler_core::ir::IrDocument> {
    let path = repo_path("test/sample.hwpx");
    let bytes = std::fs::read(&path).ok()?;
    HwpxReader.read(&bytes).ok()
}

#[test]
fn parses_real_hwpx_with_paragraphs() {
    let Some(doc) = load_sample() else {
        eprintln!("skipping: test/sample.hwpx not present");
        return;
    };
    assert!(!doc.sections.is_empty(), "expected at least one section");
    let total_paragraphs: usize = doc.sections.iter().map(|s| s.paragraphs.len()).sum();
    assert!(
        total_paragraphs > 10,
        "expected many paragraphs in a real business-plan hwpx, got {total_paragraphs}"
    );
}

#[test]
fn real_hwpx_page_dims_populate_section_properties() {
    let Some(doc) = load_sample() else {
        return;
    };
    let props = &doc.sections[0].properties;
    assert!(
        props.page_width_hwpu > 0 && props.page_height_hwpu > 0,
        "PAGE_DEF equivalent should surface page dims, got {props:?}"
    );
}

#[test]
fn real_hwpx_contains_tables() {
    let Some(doc) = load_sample() else {
        return;
    };
    let has_table = doc.sections.iter().any(|s| {
        s.paragraphs.iter().any(|p| {
            p.controls
                .iter()
                .any(|c| matches!(c.kind, ControlKind::Table(_)))
        })
    });
    assert!(has_table, "business-plan HWPX must have at least one table");
}

#[test]
fn real_hwpx_bindata_promoted_to_bin_data() {
    let Some(doc) = load_sample() else {
        return;
    };
    // BinData/image*.{png,jpg,…} gets promoted from
    // `unknown_streams` into `doc.bin_data` so the HTML preview can
    // resolve `binaryItemIDRef`. The business-plan fixture carries
    // three images — all should land here with non-empty bytes.
    assert!(
        doc.bin_data.len() >= 3,
        "expected ≥3 binary entries, got {}",
        doc.bin_data.len()
    );
    for entry in &doc.bin_data {
        assert!(!entry.bytes.is_empty(), "empty payload for {}", entry.id);
        assert!(
            entry.id.starts_with("image") || entry.id.starts_with("BIN"),
            "unexpected HWPX binary id: {}",
            entry.id
        );
    }
    // Sanity: the verbatim bucket no longer double-stores the
    // binary bytes (that was the old pre-promotion shape).
    assert!(
        !doc.unknown_streams
            .keys()
            .any(|k| k.starts_with("BinData/")),
        "BinData entries should be promoted, not duplicated"
    );
}

/// Read → write → read round-trip. Byte equality is out of scope
/// (reader drops layout / styling detail the writer can't reproduce),
/// but paragraph count and the first paragraph's text should survive
/// the XML re-emit without data loss.
#[test]
fn real_hwpx_round_trips_through_writer() {
    let Some(doc) = load_sample() else {
        return;
    };
    let bytes = HwpxWriter::default().write(&doc).expect("write");
    // Re-read the freshly emitted archive.
    let reloaded = HwpxReader.read(&bytes).expect("reload");

    assert_eq!(
        reloaded.sections.len(),
        doc.sections.len(),
        "section count changed during round-trip"
    );

    let orig_para_total: usize =
        doc.sections.iter().map(|s| s.paragraphs.len()).sum();
    let rel_para_total: usize =
        reloaded.sections.iter().map(|s| s.paragraphs.len()).sum();
    assert_eq!(
        orig_para_total, rel_para_total,
        "paragraph count changed during round-trip"
    );

    // Spot-check: first non-empty paragraph text matches across the
    // round-trip. Skips empty paragraphs since the writer may emit a
    // slightly different empty-run shape than the reader consumed.
    let first_text = |d: &hwp_transpiler_core::ir::IrDocument| -> Option<String> {
        d.sections
            .iter()
            .flat_map(|s| &s.paragraphs)
            .map(|p| p.text.clone())
            .find(|t| !t.trim().is_empty())
    };
    assert_eq!(
        first_text(&doc),
        first_text(&reloaded),
        "first non-empty paragraph text drifted"
    );
}
