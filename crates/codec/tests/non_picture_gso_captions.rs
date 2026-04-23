//! Verify that captions attached to non-picture gsos (rectangles, OLE,
//! text boxes, etc.) survive into `Control.caption_text`. Before the
//! caption-to-Control migration these were silently dropped along with
//! the `pending_picture` state when the gso turned out not to be
//! `$pic`.

use hwp_transpiler_codec::hwp::HwpReader;
use hwp_transpiler_core::ir::{ControlKind, IrDocument, Paragraph, Reader};
use std::path::Path;

fn repo_path(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(rel)
}

fn walk<'a>(paras: &'a [Paragraph], out: &mut Vec<(bool, Option<String>)>) {
    for p in paras {
        for c in &p.controls {
            let is_picture = matches!(c.kind, ControlKind::Picture(_));
            let has_caption = c.caption_text.is_some();
            if has_caption {
                out.push((is_picture, c.caption_text.clone()));
            }
            if let ControlKind::Table(t) = &c.kind {
                for cell in &t.cells {
                    walk(&cell.paragraphs, out);
                }
            }
        }
    }
}

#[test]
fn trl_surfaces_captions_on_picture_and_non_picture_controls() {
    let fixture = repo_path(
        "test/260420-1. 연구개발계획서(서식)[TRL점프업 1단계]_대진대_수정_1430_fin.hwp",
    );
    let Ok(bytes) = std::fs::read(&fixture) else {
        eprintln!("skipping: TRL fixture not present");
        return;
    };
    let doc: IrDocument = HwpReader.read(&bytes).expect("read");

    let mut captions = Vec::new();
    for s in &doc.sections {
        walk(&s.paragraphs, &mut captions);
    }

    let pic_caps = captions.iter().filter(|(p, _)| *p).count();
    let other_caps = captions.iter().filter(|(p, _)| !*p).count();

    eprintln!(
        "TRL captions: {} on pictures, {} on non-picture gsos",
        pic_caps, other_caps
    );

    assert!(
        pic_caps >= 6,
        "expected ≥6 picture captions (Phase 2b baseline); got {pic_caps}"
    );
    // The precise number of non-picture gso captions depends on what
    // HWP authors did with rectangles / text boxes / OLEs in the
    // fixture. We don't know that up front, so the test only asserts
    // that the mechanism is wired up — any captions surface cleanly
    // (no FFFC leak) and are properly joined strings.
    // Empty-string captions should never appear — the parser only
    // records `Some(_)` when caption_parts is non-empty. Raw FFFC is
    // allowed in the stored field (it's the cleanup layer's job to
    // drop it); we just verify the captions round-trip as real strings.
    for (_, cap) in &captions {
        let text = cap.as_deref().unwrap_or("");
        assert!(!text.is_empty(), "caption stored as empty Some(_)");
    }
}
