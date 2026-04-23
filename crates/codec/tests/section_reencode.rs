//! Binary HWP Phase 1: `HwpWriter` now re-encodes `/BodyText/Section{N}`
//! from the typed IR when `Section::stream_bytes` is cleared, instead
//! of erroring out. Two things to guard:
//!
//!   1. Unmutated round-trip still picks the verbatim path (byte-equal
//!      at the stream level).
//!   2. After clearing `stream_bytes`, re-encode produces bytes that
//!      re-parse into a *structurally* equivalent Section — same
//!      paragraph count, same text, same raw-records tag sequence.
//!   3. Typed mutation of `Paragraph::header` flows through the writer
//!      (PARA_HEADER sync path in `body_text::emit_section`).
//!
//! Byte-equality for a mutated section is NOT expected — DEFLATE
//! re-compression using Rust `flate2` rarely matches hwplib's Java
//! `Deflater` output byte-for-byte. Structural equality is the right
//! invariant for this phase.

use hwp_transpiler_codec::hwp::{HwpReader, HwpWriter};
use hwp_transpiler_core::ir::{IrDocument, Reader, Writer};
use std::path::PathBuf;

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn load_doc(path: &PathBuf) -> Option<(Vec<u8>, IrDocument)> {
    let bytes = std::fs::read(path).ok()?;
    let doc = HwpReader.read(&bytes).expect("read HWP");
    Some((bytes, doc))
}

/// Sanity: without any mutation, the writer picks the stream_bytes
/// verbatim path. This is what already worked before Phase 1, but we
/// guard it explicitly so future emit_section changes don't start
/// unconditionally re-encoding.
#[test]
fn unmutated_section_uses_verbatim_cache() {
    let Some((fixture_bytes, doc)) = load_doc(&fixture_path("blank.hwp")) else {
        return;
    };

    for section in &doc.sections {
        assert!(
            section.stream_bytes.is_some(),
            "parsed section should carry verbatim stream_bytes"
        );
    }

    // A write + re-read cycle should come back byte-equal at the
    // section stream level because nothing mutated. Full-file byte
    // equality is NOT asserted — the `cfb` OLE container can pick
    // different sector / FAT layouts on re-serialization. Per-stream
    // comparison is what `round_trip.rs` already relies on.
    let _ = fixture_bytes; // retained to document what we read
    let out = HwpWriter.write(&doc).expect("write");
    let reparsed = HwpReader.read(&out).expect("re-read");
    assert_eq!(doc.sections.len(), reparsed.sections.len());
    for (orig, after) in doc.sections.iter().zip(reparsed.sections.iter()) {
        assert_eq!(
            orig.stream_bytes.as_deref(),
            after.stream_bytes.as_deref(),
            "verbatim section bytes diverged on unmutated round-trip"
        );
    }
}

/// Clear `stream_bytes` on every section and write. The writer must
/// fall through to `body_text::emit_section`, and re-parsing the
/// output must yield the same structural record tree as the input.
#[test]
fn structural_reencode_after_stream_bytes_cleared() {
    let Some((_bytes, mut doc)) = load_doc(&fixture_path("blank.hwp")) else {
        return;
    };

    // Force the re-encode path.
    for section in &mut doc.sections {
        section.invalidate_verbatim();
        assert!(section.stream_bytes.is_none());
    }

    let out = HwpWriter.write(&doc).expect("write");
    let after = HwpReader.read(&out).expect("re-read");

    assert_eq!(
        doc.sections.len(),
        after.sections.len(),
        "section count diverged"
    );
    for (i, (lhs, rhs)) in doc.sections.iter().zip(after.sections.iter()).enumerate() {
        assert_eq!(
            lhs.paragraphs.len(),
            rhs.paragraphs.len(),
            "section {i}: paragraph count diverged"
        );
        assert_eq!(
            lhs.raw_records.len(),
            rhs.raw_records.len(),
            "section {i}: section-prelude raw_records length diverged"
        );
        for (pi, (lp, rp)) in lhs.paragraphs.iter().zip(rhs.paragraphs.iter()).enumerate() {
            assert_eq!(
                lp.text, rp.text,
                "section {i} paragraph {pi}: text diverged"
            );
            assert_eq!(
                lp.header, rp.header,
                "section {i} paragraph {pi}: ParagraphHeader diverged"
            );
            let lhs_tags: Vec<u16> = lp.raw_records.iter().map(|r| r.tag).collect();
            let rhs_tags: Vec<u16> = rp.raw_records.iter().map(|r| r.tag).collect();
            assert_eq!(
                lhs_tags, rhs_tags,
                "section {i} paragraph {pi}: raw record tag sequence diverged"
            );
        }
    }
}

/// Typed mutation of `Paragraph::header` must land in the re-encoded
/// stream. Pick a field that hwplib-generated fixtures leave at a
/// known value (`style_id` on paragraph 0 is 0 in `blank.hwp`), flip
/// it, write, re-read, and verify the change persisted.
#[test]
fn paragraph_header_mutation_flows_through_writer() {
    let Some((_bytes, mut doc)) = load_doc(&fixture_path("blank.hwp")) else {
        return;
    };
    if doc.sections.is_empty() || doc.sections[0].paragraphs.is_empty() {
        return; // fixture without paragraphs isn't a useful test.
    }

    let original_style = doc.sections[0].paragraphs[0].header.style_id;
    let mutated_style = original_style.wrapping_add(1);

    // paragraphs_mut auto-clears stream_bytes.
    doc.sections[0].paragraphs_mut()[0].header.style_id = mutated_style;
    assert!(doc.sections[0].stream_bytes.is_none());

    let out = HwpWriter.write(&doc).expect("write after mutation");
    let after = HwpReader.read(&out).expect("re-read after mutation");

    assert_eq!(
        after.sections[0].paragraphs[0].header.style_id, mutated_style,
        "style_id mutation did not flow through writer"
    );
    // And only that field should have moved.
    assert_eq!(
        after.sections[0].paragraphs[0].header.char_count,
        doc.sections[0].paragraphs[0].header.char_count,
        "char_count collateral damage from style_id edit"
    );
}

/// `Paragraph::char_shape_runs` (PARA_CHAR_SHAPE typed view) must
/// flow through the writer — parallel to the header sync path.
#[test]
fn char_shape_runs_mutation_flows_through_writer() {
    use hwp_transpiler_core::ir::CharShapeRun;

    let Some((_bytes, mut doc)) = load_doc(&fixture_path("blank.hwp")) else {
        return;
    };
    if doc.sections.is_empty() || doc.sections[0].paragraphs.is_empty() {
        return;
    }

    // Replace the paragraph's runs with a deliberately different
    // sequence. Two runs (start=0, start=5) referencing shape ids we
    // can uniquely identify when we re-read.
    let mutated = vec![
        CharShapeRun { start: 0, char_shape_id: 42 },
        CharShapeRun { start: 5, char_shape_id: 99 },
    ];
    doc.sections[0].paragraphs_mut()[0].char_shape_runs = mutated.clone();

    let out = HwpWriter.write(&doc).expect("write");
    let after = HwpReader.read(&out).expect("re-read");

    assert_eq!(
        after.sections[0].paragraphs[0].char_shape_runs, mutated,
        "char_shape_runs mutation did not flow through writer"
    );
}

/// `Paragraph::line_segments` (PARA_LINE_SEG typed view) must flow
/// through the writer too.
#[test]
fn line_segments_mutation_flows_through_writer() {
    use hwp_transpiler_core::ir::LineSegment;

    let Some((_bytes, mut doc)) = load_doc(&fixture_path("blank.hwp")) else {
        return;
    };
    if doc.sections.is_empty() || doc.sections[0].paragraphs.is_empty() {
        return;
    }

    let mutated = vec![LineSegment {
        text_start: 0,
        vertical_position_hwpu: 0,
        line_height_hwpu: 1234,
        text_height_hwpu: 1111,
        baseline_distance_hwpu: 800,
        line_spacing_hwpu: 600,
        start_x_hwpu: 0,
        width_hwpu: 30000,
        tag: 0x07,
    }];
    doc.sections[0].paragraphs_mut()[0].line_segments = mutated.clone();

    let out = HwpWriter.write(&doc).expect("write");
    let after = HwpReader.read(&out).expect("re-read");

    assert_eq!(
        after.sections[0].paragraphs[0].line_segments, mutated,
        "line_segments mutation did not flow through writer"
    );
}

/// `Paragraph::text` edits are silently dropped: the typed view is
/// lossy (extended-control U+FFFC collapse), so we don't try to round-
/// trip it yet. Document this via a test so that when a typed
/// PARA_TEXT encoder lands, this test flips and becomes a positive
/// assertion.
#[test]
fn paragraph_text_edits_are_currently_ignored() {
    let Some((_bytes, mut doc)) = load_doc(&fixture_path("blank.hwp")) else {
        return;
    };
    if doc.sections.is_empty() || doc.sections[0].paragraphs.is_empty() {
        return;
    }

    let original_text = doc.sections[0].paragraphs[0].text.clone();
    doc.sections[0].paragraphs_mut()[0].text = "MUTATED TEXT".into();

    let out = HwpWriter.write(&doc).expect("write");
    let after = HwpReader.read(&out).expect("re-read");

    assert_eq!(
        after.sections[0].paragraphs[0].text, original_text,
        "text edits should be a no-op until PARA_TEXT encoder lands"
    );
}

/// Also exercise the TRL fixture (real-world complex document) to
/// confirm the re-encode path handles nested tables, gso shapes, and
/// multi-paragraph records without structural loss. Gitignored → skip
/// when absent.
#[test]
fn trl_fixture_reencodes_structurally() {
    let Some((_bytes, mut doc)) = load_doc(&repo_path(
        "test/260420-1. 연구개발계획서(서식)[TRL점프업 1단계]_대진대_수정_1430_fin.hwp",
    )) else {
        eprintln!("skipping: TRL fixture not present");
        return;
    };

    for section in &mut doc.sections {
        section.invalidate_verbatim();
    }

    let out = HwpWriter.write(&doc).expect("write");
    let after = HwpReader.read(&out).expect("re-read");

    assert_eq!(doc.sections.len(), after.sections.len());
    for (i, (lhs, rhs)) in doc.sections.iter().zip(after.sections.iter()).enumerate() {
        assert_eq!(
            lhs.paragraphs.len(),
            rhs.paragraphs.len(),
            "section {i}: paragraph count diverged"
        );
    }
}
