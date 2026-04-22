//! End-to-end asset pipeline check: every `![](prefix/…)` link the
//! Markdown exporter emits must point at a real file produced by
//! `assets::dump_assets`. Catches regressions where the filename
//! convention used by `emit_picture` drifts away from the OLE stream
//! naming convention reader hands back as `BinaryEntry.id`.

use hwp_transpiler_codec::export::{assets, markdown};
use hwp_transpiler_codec::hwp::HwpReader;
use hwp_transpiler_core::ir::Reader;
use std::path::Path;

fn repo_path(rel: &str) -> std::path::PathBuf {
    // `CARGO_MANIFEST_DIR` is the crate root (`crates/codec`); we want
    // the workspace root two levels up so test paths stay stable
    // regardless of where `cargo test` is invoked from.
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().unwrap().parent().unwrap().join(rel)
}

#[test]
fn trl_markdown_links_resolve_to_dumped_files() {
    let fixture_path = repo_path(
        "test/260420-1. 연구개발계획서(서식)[TRL점프업 1단계]_대진대_수정_1430_fin.hwp",
    );
    let Ok(bytes) = std::fs::read(&fixture_path) else {
        eprintln!("skipping: TRL fixture not present at {}", fixture_path.display());
        return;
    };
    let doc = HwpReader.read(&bytes).expect("read");

    let tmp = std::env::temp_dir().join(format!(
        "hwp-transpiler-xref-{}",
        std::process::id()
    ));
    std::fs::remove_dir_all(&tmp).ok();
    let assets_dir = tmp.join("out.assets");

    // Dump assets first.
    let written = assets::dump_assets(&doc, &assets_dir).expect("dump");
    assert!(
        !written.is_empty(),
        "TRL fixture must produce at least one asset"
    );

    // Emit Markdown pointed at the same prefix the CLI would use for a
    // sibling layout.
    let md = markdown::to_markdown_with(
        &doc,
        &markdown::MdOptions {
            assets_path: Some("out.assets".into()),
            ..markdown::MdOptions::default()
        },
    );

    // Every emitted image reference must correspond to a file on disk.
    // The exporter currently only emits top-level pictures, so the count
    // of links is a subset of `written` — but each referenced file must
    // exist.
    let mut refs = 0;
    for line in md.lines() {
        if let Some(rest) = line.strip_prefix("![](") {
            let Some(end) = rest.find(')') else { continue };
            let url = &rest[..end];
            refs += 1;
            let filename = url
                .strip_prefix("out.assets/")
                .unwrap_or_else(|| panic!("link missing prefix: {url}"));
            let abs = assets_dir.join(filename);
            assert!(
                abs.is_file(),
                "MD link {url} has no backing file at {}",
                abs.display()
            );
        }
    }
    assert!(
        refs >= 1,
        "TRL fixture has ≥1 top-level picture; MD emitted none"
    );

    std::fs::remove_dir_all(&tmp).ok();
}

#[test]
fn no_assets_mode_emits_no_image_links() {
    // Same fixture, but with `assets_path: None` — placeholder-only path.
    // Regression guard: someone adding a picture-link code path without
    // checking the option would surface here.
    let fixture_path = repo_path(
        "test/260420-1. 연구개발계획서(서식)[TRL점프업 1단계]_대진대_수정_1430_fin.hwp",
    );
    let Ok(bytes) = std::fs::read(&fixture_path) else {
        eprintln!("skipping: TRL fixture not present");
        return;
    };
    let doc = HwpReader.read(&bytes).expect("read");

    let md = markdown::to_markdown(&doc);
    for (i, line) in md.lines().enumerate() {
        assert!(
            !line.contains("!["),
            "unexpected image link at line {i}: {line}"
        );
    }
}
