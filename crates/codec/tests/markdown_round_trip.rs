//! MD → IR → MD round-trip. Proves the import side's synthesised
//! Style table is shaped exactly the way the existing exporter's
//! `heading_level` lookup expects, so a doc authored as Markdown
//! can flow through the IR (and from there into HWP / HWPX writers)
//! and round-trip back to equivalent Markdown.

use hwp_transpiler_codec::export::markdown::to_markdown;
use hwp_transpiler_codec::import::markdown::from_markdown;

fn round_trip(src: &str) -> String {
    let doc = from_markdown(src).expect("import");
    // The exporter stamps a `<!-- hwp-transpiler: format=human -->`
    // header so the importer can dispatch deterministically. Strip
    // it here so the structural assertions stay focused on the body.
    let raw = to_markdown(&doc);
    let header = "<!-- hwp-transpiler: format=human -->\n";
    raw.strip_prefix(header).map(|s| s.to_string()).unwrap_or(raw)
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
fn simple_table_round_trips_via_existing_exporter() {
    let md = "| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |";
    let doc = from_markdown(md).expect("import");
    // Sanity: re-exporting must produce a pipe table that contains
    // the four data cells. The exact whitespace depends on the
    // exporter's column-width heuristic, so don't byte-compare.
    let back = to_markdown(&doc);
    assert!(back.contains("| A"), "header A: {back}");
    assert!(back.contains("| B"), "header B: {back}");
    assert!(back.contains("| 1"), "cell 1,1: {back}");
    assert!(back.contains("| 4"), "cell 2,2: {back}");
    // Markdown table separator line.
    assert!(back.contains("|---") || back.contains("| ---"),
        "separator row present: {back}");
}

#[test]
fn body_then_table_then_body_round_trips_each_block() {
    let md = "Intro paragraph.\n\n| K | V |\n|---|---|\n| name | foo |\n\nClosing.";
    let doc = from_markdown(md).expect("import");
    let back = to_markdown(&doc);
    assert!(back.contains("Intro paragraph."), "lead body: {back}");
    assert!(back.contains("| K"), "table header: {back}");
    assert!(back.contains("| name"), "table cell: {back}");
    assert!(back.contains("Closing."), "trailing body: {back}");
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
