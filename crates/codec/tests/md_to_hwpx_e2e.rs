//! End-to-end MD → HWPX (skeleton-bundled) → re-read round-trip.
//! Proves the writer's output is parseable by our HwpxReader and
//! that the surgical header rewriter places IR-side paraShapes /
//! styles into the bundled skeleton's `<hh:paraProperties>`
//! container, ready for round-trip back to Markdown.

use hwp_transpiler_codec::hwpx::skeleton::bundle_default_skeleton;
use hwp_transpiler_codec::hwpx::{HwpxReader, HwpxWriter};
use hwp_transpiler_codec::import::markdown::from_markdown;
use hwp_transpiler_core::ir::{ControlKind, Reader, Writer};

fn md_to_hwpx_bytes(src: &str) -> Vec<u8> {
    let mut doc = from_markdown(src).expect("import");
    bundle_default_skeleton(&mut doc);
    HwpxWriter::default().write(&doc).expect("write")
}

#[test]
fn body_paragraph_round_trips_through_writer_and_reader() {
    let bytes = md_to_hwpx_bytes("Hello world");
    assert_eq!(&bytes[0..4], b"PK\x03\x04", "valid ZIP magic");

    let doc = HwpxReader.read(&bytes).expect("re-read");
    assert_eq!(doc.sections.len(), 1);
    // `paragraphs[0]` is the synthetic secPr paragraph (no text);
    // user content lands in subsequent paragraphs.
    let para = doc.sections[0]
        .paragraphs
        .iter()
        .find(|p| p.text == "Hello world")
        .expect("body paragraph survived round-trip");
    assert_eq!(para.text, "Hello world");
}

#[test]
fn heading_paragraph_carries_para_shape_ref_through_writer() {
    // Phase-2 round-trip: the heading paragraph's `paraPrIDRef`
    // attribute survives writer + reader, and the rewriter inserts
    // the IR-side paraShape (id 1) into the bundled skeleton's empty
    // `<hh:paraProperties>`. The `heading_level` bits themselves are
    // a HWP5-only concept that doesn't have a `<hh:paraPr>`
    // attribute in HWPX — full heading-level round-trip needs the
    // `<hh:styles>` container path, which is a follow-up TODO.
    let bytes = md_to_hwpx_bytes("# 제목");
    let doc = HwpxReader.read(&bytes).expect("re-read");
    let para = doc.sections[0]
        .paragraphs
        .iter()
        .find(|p| p.text == "제목")
        .expect("heading paragraph survived round-trip");
    let para_shape_id = para.header.para_shape_id as usize;
    assert!(
        para_shape_id >= 1 && para_shape_id <= 6,
        "para_shape_id was {para_shape_id}, paraShapes.len()={}",
        doc.doc_info.para_shapes.len()
    );
    // Slot exists in the rebuilt para_shapes table.
    assert!(doc.doc_info.para_shapes.len() > para_shape_id);
}

#[test]
fn pipe_table_round_trips_as_table_control() {
    let md = "| K | V |\n|---|---|\n| name | foo |\n| size | 42 |";
    let bytes = md_to_hwpx_bytes(md);
    let doc = HwpxReader.read(&bytes).expect("re-read");

    // The table lives in some paragraph's controls. Find it.
    let table = doc
        .sections[0]
        .paragraphs
        .iter()
        .flat_map(|p| &p.controls)
        .find_map(|c| match &c.kind {
            ControlKind::Table(t) => Some(t),
            _ => None,
        })
        .expect("table control survived round-trip");

    assert_eq!(table.cols, 2);
    assert_eq!(table.rows, 3); // header + 2 body rows
    assert_eq!(table.cells.len(), 6);
    let texts: Vec<&str> = table
        .cells
        .iter()
        .map(|c| c.paragraphs[0].text.as_str())
        .collect();
    assert!(texts.contains(&"K"));
    assert!(texts.contains(&"name"));
    assert!(texts.contains(&"42"));
}

#[test]
fn skeleton_meta_inf_present_in_output_archive() {
    let bytes = md_to_hwpx_bytes("Hello");
    // Cheap-but-effective check: the bundled skeleton's distinct
    // strings appear in the ZIP body. zip stores parts with their
    // path as a local-file-header field, so the bytes-level check
    // catches the typical missing-skeleton regression.
    let s = String::from_utf8_lossy(&bytes);
    assert!(s.contains("META-INF/container.xml"), "container manifest");
    assert!(s.contains("Contents/content.hpf"), "package manifest");
    assert!(s.contains("Contents/header.xml"), "header skeleton");
}
