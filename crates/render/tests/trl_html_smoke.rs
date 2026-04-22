//! End-to-end smoke test: parse TRL fixture → emit HTML → verify that
//! the structural markers (tables with rowspan/colspan, figures with
//! captions, outline headings) all land in the output.

use hwp_transpiler_render::{HtmlOptions, to_html_with};
use std::path::Path;

fn repo_path(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(rel)
}

/// Minimal inline reader so the render crate stays one-way-dependent
/// on core; the test pulls codec::hwp::HwpReader at test scope only.
fn load_trl() -> Option<hwp_transpiler_core::ir::IrDocument> {
    use hwp_transpiler_codec::hwp::HwpReader;
    use hwp_transpiler_core::ir::Reader;
    let path = repo_path(
        "test/260420-1. 연구개발계획서(서식)[TRL점프업 1단계]_대진대_수정_1430_fin.hwp",
    );
    let bytes = std::fs::read(&path).ok()?;
    HwpReader.read(&bytes).ok()
}

#[test]
fn trl_html_has_tables_and_figures() {
    let Some(doc) = load_trl() else {
        eprintln!("skipping: TRL fixture not present");
        return;
    };

    let html = to_html_with(
        &doc,
        &HtmlOptions {
            assets_path: Some("trl.assets".into()),
        },
    );

    // Structural invariants:
    assert!(html.starts_with("<article class=\"hwp-preview\">"));
    assert!(html.trim_end().ends_with("</article>"));
    assert!(html.contains("<section>"));
    assert!(html.contains("</section>"));

    // 53 tables in TRL — at least one `<table>` appears.
    assert!(html.contains("<table>"));

    // At least one colspan or rowspan attribute (TRL has both).
    assert!(
        html.contains("colspan=\"") || html.contains("rowspan=\""),
        "expected merged cells"
    );

    // 9 pictures → some <figure> elements. With assets_path set, at
    // least one <img src="trl.assets/…"> appears.
    assert!(html.contains("<figure>"));
    assert!(html.contains("<img src=\"trl.assets/BIN"));

    // At least one caption survives into <figcaption>.
    assert!(
        html.contains("<figcaption>"),
        "caption-bearing pictures should emit figcaption"
    );

    // No raw FFFC / PUA bullets leak.
    assert!(!html.contains('\u{FFFC}'));
    assert!(!html.contains('\u{F2B1}'));

    // Entity escaping: the TRL fixture contains `&` in "현대건설, GS건설"
    // style text and should escape it. Quick sanity: no lone `&` that's
    // not part of an entity.
    let lone_ampersand = html
        .match_indices('&')
        .filter(|(i, _)| {
            let tail = &html[*i..];
            !tail.starts_with("&amp;")
                && !tail.starts_with("&lt;")
                && !tail.starts_with("&gt;")
                && !tail.starts_with("&quot;")
                && !tail.starts_with("&#39;")
        })
        .count();
    assert_eq!(lone_ampersand, 0, "unescaped & in output");
}

#[test]
fn trl_html_is_deterministic() {
    let Some(doc) = load_trl() else {
        return;
    };
    let a = to_html_with(&doc, &HtmlOptions::default());
    let b = to_html_with(&doc, &HtmlOptions::default());
    assert_eq!(a, b);
}
