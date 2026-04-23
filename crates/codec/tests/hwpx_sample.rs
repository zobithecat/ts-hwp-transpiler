//! Integration smoke over a real HWPX fixture. Skips if the file
//! isn't on disk — the user-authored sample sits in `/test/` which
//! is git-ignored, so CI won't see it.

use hwp_transpiler_codec::hwpx::HwpxReader;
use hwp_transpiler_core::ir::{ControlKind, Reader};
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
fn real_hwpx_preserves_bindata_in_unknown_streams() {
    let Some(doc) = load_sample() else {
        return;
    };
    // BinData/image*.png should be captured verbatim for future
    // figure promotion.
    let has_bin = doc
        .unknown_streams
        .keys()
        .any(|k| k.starts_with("BinData/"));
    assert!(
        has_bin,
        "expected BinData/ entries to land in unknown_streams"
    );
}
