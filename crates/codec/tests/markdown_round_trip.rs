//! MD → IR → MD round-trip. Proves the import side's synthesised
//! Style table is shaped exactly the way the existing exporter's
//! `heading_level` lookup expects, so a doc authored as Markdown
//! can flow through the IR (and from there into HWP / HWPX writers)
//! and round-trip back to equivalent Markdown.

use hwp_transpiler_codec::export::markdown::to_markdown;
use hwp_transpiler_codec::import::markdown::from_markdown;

fn round_trip(src: &str) -> String {
    let doc = from_markdown(src).expect("import");
    to_markdown(&doc)
}

/// Strip trailing whitespace + collapse runs of blank lines so the
/// assertion focuses on structural equivalence rather than exact
/// blank-line spacing.
fn normalise(s: &str) -> String {
    let mut out = String::new();
    let mut blank_streak = 0;
    for line in s.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            blank_streak += 1;
            if blank_streak < 2 {
                out.push('\n');
            }
        } else {
            blank_streak = 0;
            out.push_str(trimmed);
            out.push('\n');
        }
    }
    out.trim_end().to_string()
}

#[test]
fn body_paragraph_round_trips() {
    let md = "Hello world.";
    let back = round_trip(md);
    assert_eq!(normalise(&back), "Hello world.");
}

#[test]
fn heading_level_one_round_trips_with_marker() {
    let md = "# 제목";
    let back = round_trip(md);
    assert_eq!(normalise(&back), "# 제목");
}

#[test]
fn heading_levels_round_trip_distinctly() {
    let md = "# A\n\n## B\n\n### C";
    let back = round_trip(md);
    assert_eq!(normalise(&back), normalise(md));
}

#[test]
fn mixed_heading_and_body_preserves_order() {
    let md = "# 본문 시작\n\n첫 단락입니다.\n\n## 절\n\n두 번째 단락.";
    let back = round_trip(md);
    assert_eq!(normalise(&back), normalise(md));
}

#[test]
fn empty_input_round_trips_to_empty() {
    let back = round_trip("");
    assert!(
        back.trim().is_empty(),
        "expected empty MD output, got: {back:?}"
    );
}

#[test]
fn multi_line_body_paragraph_collapses_to_single_line() {
    // Soft breaks become spaces in the IR's paragraph text. A
    // re-export emits a single line. Acceptable round-trip — the
    // semantic ("one paragraph") is preserved.
    let md = "first line\nsecond line";
    let back = round_trip(md);
    assert_eq!(normalise(&back), "first line second line");
}
