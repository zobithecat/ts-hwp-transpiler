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

/// Unmutated round-trip: the surgical header rewriter must preserve
/// the parsed DocInfo shapes semantically. If we re-emit the four
/// known sections and re-parse them, the IR shapes should match the
/// original IR exactly.
#[test]
fn real_hwpx_unmutated_round_trip_preserves_doc_info_shapes() {
    let Some(doc) = load_sample() else {
        return;
    };
    let bytes = HwpxWriter::default().write(&doc).expect("write");
    let reloaded = HwpxReader.read(&bytes).expect("reload");

    assert_eq!(
        doc.doc_info.font_faces.hangul.len(),
        reloaded.doc_info.font_faces.hangul.len(),
        "fontface count drifted"
    );
    for (orig, back) in doc
        .doc_info
        .font_faces
        .hangul
        .iter()
        .zip(reloaded.doc_info.font_faces.hangul.iter())
    {
        assert_eq!(orig.name, back.name, "fontface name drifted");
    }
    assert_eq!(
        doc.doc_info.para_shapes.len(),
        reloaded.doc_info.para_shapes.len(),
        "paraShape count drifted"
    );
    for (i, (orig, back)) in doc
        .doc_info
        .para_shapes
        .iter()
        .zip(reloaded.doc_info.para_shapes.iter())
        .enumerate()
    {
        assert_eq!(
            orig.align(),
            back.align(),
            "paraShape[{i}] align drifted"
        );
    }
    assert_eq!(
        doc.doc_info.char_shapes.len(),
        reloaded.doc_info.char_shapes.len(),
        "charShape count drifted"
    );
    for (i, (orig, back)) in doc
        .doc_info
        .char_shapes
        .iter()
        .zip(reloaded.doc_info.char_shapes.iter())
        .enumerate()
    {
        assert_eq!(
            orig.base_size, back.base_size,
            "charShape[{i}] base_size drifted"
        );
        assert_eq!(orig.color, back.color, "charShape[{i}] color drifted");
        assert_eq!(orig.bold(), back.bold(), "charShape[{i}] bold drifted");
        assert_eq!(orig.italic(), back.italic(), "charShape[{i}] italic drifted");
        assert_eq!(orig.strike(), back.strike(), "charShape[{i}] strike drifted");
    }
    assert_eq!(
        doc.doc_info.border_fills.len(),
        reloaded.doc_info.border_fills.len(),
        "borderFill count drifted"
    );
    for (i, (orig, back)) in doc
        .doc_info
        .border_fills
        .iter()
        .zip(reloaded.doc_info.border_fills.iter())
        .enumerate()
    {
        assert_eq!(
            orig.fill.back_color(),
            back.fill.back_color(),
            "borderFill[{i}] colour drifted"
        );
    }
}

/// Mutation flow: editing `ParaShape.attribute` (align bits) on the
/// IR before write must surface in the re-parsed document.
#[test]
fn mutating_para_shape_align_flows_through_writer() {
    let Some(mut doc) = load_sample() else {
        return;
    };
    // Find a paraShape that isn't already RIGHT (align bits == 2).
    let target_idx = doc
        .doc_info
        .para_shapes
        .iter()
        .position(|p| p.align() != 2);
    let Some(idx) = target_idx else {
        eprintln!("skipping: every paraShape is already RIGHT");
        return;
    };
    let original_align = doc.doc_info.para_shapes[idx].align();
    // Force align bits → 2 (RIGHT).
    let attr = &mut doc.doc_info.para_shapes[idx].attribute;
    *attr = (*attr & !0x07) | 0x02;
    assert_ne!(original_align, 2, "test pre-condition");

    let bytes = HwpxWriter::default().write(&doc).expect("write");
    let reloaded = HwpxReader.read(&bytes).expect("reload");

    assert_eq!(
        reloaded.doc_info.para_shapes[idx].align(),
        2,
        "paraShape[{idx}] align mutation lost on round-trip"
    );
}

/// Mutation flow: editing `CharShape.color` on the IR before write
/// must surface in the re-parsed document.
#[test]
fn mutating_char_shape_color_flows_through_writer() {
    let Some(mut doc) = load_sample() else {
        return;
    };
    if doc.doc_info.char_shapes.is_empty() {
        return;
    }
    let idx = 0;
    let new_color = 0x0034_5678; // arbitrary, distinct from typical defaults
    doc.doc_info.char_shapes[idx].color = new_color;
    doc.doc_info.char_shapes[idx].base_size = 1700;

    let bytes = HwpxWriter::default().write(&doc).expect("write");
    let reloaded = HwpxReader.read(&bytes).expect("reload");

    assert_eq!(
        reloaded.doc_info.char_shapes[idx].color, new_color,
        "charShape color mutation lost"
    );
    assert_eq!(
        reloaded.doc_info.char_shapes[idx].base_size, 1700,
        "charShape base_size mutation lost"
    );
}

/// Mutation flow: editing multi-script CharShape arrays (font_ids,
/// ratios, rel_sizes, char_spacings, char_offsets) on the IR before
/// write must surface in the re-parsed document.
#[test]
fn mutating_char_shape_multi_script_arrays_flows_through_writer() {
    let Some(mut doc) = load_sample() else {
        return;
    };
    if doc.doc_info.char_shapes.is_empty() {
        return;
    }
    let idx = 0;
    doc.doc_info.char_shapes[idx].font_ids = [7, 9, 0, 0, 0, 0, 0];
    doc.doc_info.char_shapes[idx].ratios = [120, 90, 100, 100, 100, 100, 100];
    doc.doc_info.char_shapes[idx].char_spacings = [-3, 5, 0, 0, 0, 0, 0];
    doc.doc_info.char_shapes[idx].char_offsets = [2, -1, 0, 0, 0, 0, 0];

    let bytes = HwpxWriter::default().write(&doc).expect("write");
    let reloaded = HwpxReader.read(&bytes).expect("reload");

    assert_eq!(
        reloaded.doc_info.char_shapes[idx].font_ids,
        [7, 9, 0, 0, 0, 0, 0],
        "font_ids mutation lost"
    );
    assert_eq!(
        reloaded.doc_info.char_shapes[idx].ratios,
        [120, 90, 100, 100, 100, 100, 100],
        "ratios mutation lost"
    );
    assert_eq!(
        reloaded.doc_info.char_shapes[idx].char_spacings,
        [-3, 5, 0, 0, 0, 0, 0],
        "char_spacings mutation lost"
    );
    assert_eq!(
        reloaded.doc_info.char_shapes[idx].char_offsets,
        [2, -1, 0, 0, 0, 0, 0],
        "char_offsets mutation lost"
    );
}

/// Mutation flow: toggling bold / italic on a CharShape (presence-only
/// children in HWPX) must surface in the re-parsed document. Picks
/// a charShape that doesn't already have both bold and italic.
#[test]
fn toggling_bold_italic_flows_through_writer() {
    let Some(mut doc) = load_sample() else {
        return;
    };
    if doc.doc_info.char_shapes.is_empty() {
        return;
    }
    // Pick the first charShape that's not already both-on. Force
    // both bold and italic bits on.
    let target_idx = doc
        .doc_info
        .char_shapes
        .iter()
        .position(|cs| !cs.bold() || !cs.italic());
    let Some(idx) = target_idx else {
        eprintln!("skipping: every charShape already has bold+italic");
        return;
    };
    doc.doc_info.char_shapes[idx].attr |= 0x0000_0003; // bold + italic

    let bytes = HwpxWriter::default().write(&doc).expect("write");
    let reloaded = HwpxReader.read(&bytes).expect("reload");

    assert!(
        reloaded.doc_info.char_shapes[idx].bold(),
        "bold mutation lost on charShape[{idx}]"
    );
    assert!(
        reloaded.doc_info.char_shapes[idx].italic(),
        "italic mutation lost on charShape[{idx}]"
    );
}

/// Mutation flow: turning bold OFF on a charShape that originally had
/// it must surface in the re-parsed document — the writer needs to
/// drop the original `<hh:bold/>` event from output.
#[test]
fn turning_bold_off_flows_through_writer() {
    let Some(mut doc) = load_sample() else {
        return;
    };
    let target_idx = doc.doc_info.char_shapes.iter().position(|cs| cs.bold());
    let Some(idx) = target_idx else {
        eprintln!("skipping: no charShape originally has bold on");
        return;
    };
    doc.doc_info.char_shapes[idx].attr &= !0x0000_0002;

    let bytes = HwpxWriter::default().write(&doc).expect("write");
    let reloaded = HwpxReader.read(&bytes).expect("reload");

    assert!(
        !reloaded.doc_info.char_shapes[idx].bold(),
        "bold-off mutation lost on charShape[{idx}]"
    );
}

/// Mutation flow: pushing a new ParaShape onto the IR vec must
/// surface as a fresh `<hh:paraPr>` block before the End tag of
/// `<hh:paraProperties>`.
#[test]
fn pushing_new_para_shape_flows_through_writer() {
    let Some(mut doc) = load_sample() else {
        return;
    };
    let original_count = doc.doc_info.para_shapes.len();
    let mut new_shape = hwp_transpiler_core::ir::ParaShape::default();
    new_shape.attribute = 3; // CENTER
    doc.doc_info.para_shapes.push(new_shape);

    let bytes = HwpxWriter::default().write(&doc).expect("write");
    let reloaded = HwpxReader.read(&bytes).expect("reload");

    assert_eq!(
        reloaded.doc_info.para_shapes.len(),
        original_count + 1,
        "added paraShape did not appear"
    );
    assert_eq!(
        reloaded.doc_info.para_shapes[original_count].align(),
        3,
        "added paraShape align lost"
    );
}

/// Mutation flow: pushing a new CharShape must surface fully (parent
/// attrs + multi-script arrays + bold flag).
#[test]
fn pushing_new_char_shape_flows_through_writer() {
    let Some(mut doc) = load_sample() else {
        return;
    };
    let original_count = doc.doc_info.char_shapes.len();
    let mut cs = hwp_transpiler_core::ir::CharShape::default();
    cs.base_size = 1700;
    cs.color = 0x0000_00FF; // red
    cs.attr = 0x0000_0002; // bold
    cs.font_ids = [4, 4, 0, 0, 0, 0, 0];
    cs.ratios = [100, 100, 100, 100, 100, 100, 100];
    cs.rel_sizes = [100, 100, 100, 100, 100, 100, 100];
    doc.doc_info.char_shapes.push(cs);

    let bytes = HwpxWriter::default().write(&doc).expect("write");
    let reloaded = HwpxReader.read(&bytes).expect("reload");

    assert_eq!(
        reloaded.doc_info.char_shapes.len(),
        original_count + 1,
        "added charShape did not appear"
    );
    let new_idx = original_count;
    assert_eq!(reloaded.doc_info.char_shapes[new_idx].base_size, 1700);
    assert_eq!(reloaded.doc_info.char_shapes[new_idx].color, 0x0000_00FF);
    assert!(reloaded.doc_info.char_shapes[new_idx].bold());
    assert_eq!(reloaded.doc_info.char_shapes[new_idx].font_ids[0], 4);
}

/// Mutation flow: editing `FontFace.name` on the IR before write must
/// surface in the re-parsed document.
#[test]
fn mutating_font_face_name_flows_through_writer() {
    let Some(mut doc) = load_sample() else {
        return;
    };
    if doc.doc_info.font_faces.hangul.is_empty() {
        return;
    }
    doc.doc_info.font_faces.hangul[0].name = "RoundTripFont".into();

    let bytes = HwpxWriter::default().write(&doc).expect("write");
    let reloaded = HwpxReader.read(&bytes).expect("reload");

    assert_eq!(
        reloaded.doc_info.font_faces.hangul[0].name, "RoundTripFont",
        "fontface name mutation lost"
    );
}
