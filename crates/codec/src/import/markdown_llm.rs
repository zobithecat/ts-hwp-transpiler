//! LLM-mode Markdown → [`IrDocument`] importer.
//!
//! Inverse of `export::markdown_llm`. The LLM emit path produces a
//! line-record format that's still UTF-8 Markdown but record-shaped
//! rather than prose-shaped:
//!
//! ```text
//! SECTION[id=sec-0]
//!
//! PARAGRAPH[id=par-s0-p0]
//! TEXT: 본문 단락 텍스트.
//!
//! TABLE[id=tbl-s0-p1-c0,rows=3,cols=2,kind=schedule]
//! CELL[id=cell-…,row=0,col=0,rowspan=1,colspan=1,role=header]
//! TEXT[par-…-r0c0-p0]: 항목
//! …
//! END TABLE[tbl-s0-p1-c0]
//! ```
//!
//! GFM tables go through `import::markdown::from_markdown`; this path
//! handles the alternative LLM emit. `from_llm_markdown` is the
//! direct entrypoint; `markdown::from_markdown` auto-detects via the
//! leading `SECTION[id=` sigil and routes here.
//!
//! Phase-1 scope of this slice: `SECTION`, `PARAGRAPH`, `TABLE`,
//! `CELL`, `TEXT`, and `END TABLE` records. Figure / caption /
//! equation records aren't yet decoded — they fall through as
//! ignored lines, which is graceful but lossy.

use std::collections::HashMap;

use hwp_transpiler_core::ir::{
    CharShapeRun, Control, ControlKind, IrDocument, IrError, Paragraph, ParagraphHeader,
    PictureControl, Section, TableCell, TableControl,
};

use crate::asset_pipeline::decode_data_uri_to_binary_entry;

use base64::{engine::general_purpose::STANDARD, Engine as _};

/// Decode the `data:application/octet-stream;base64,…` URI emitted
/// alongside `SECTION_BYTES` records. Returns `None` for any non-
/// base64 / non-data scheme — the caller drops the section bytes
/// silently in that case rather than corrupting `stream_bytes`.
fn decode_octet_stream_data_uri(uri: &str) -> Option<Vec<u8>> {
    let rest = uri.strip_prefix("data:")?;
    let (header, payload) = rest.split_once(',')?;
    let (_mime, encoding) = header
        .split_once(';')
        .map(|(m, e)| (m.trim(), e.trim()))
        .unwrap_or((header.trim(), ""));
    if encoding != "base64" {
        return None;
    }
    STANDARD.decode(payload).ok()
}

/// Returns true when `src` is recognisably the LLM-mode export
/// format. Uses the sigil our exporter emits at the top of every
/// document — `SECTION[id=sec-…]` as the first non-blank line.
pub fn looks_like_llm_format(src: &str) -> bool {
    src.lines()
        .map(|l| l.trim_start())
        .find(|l| !l.is_empty())
        .map(|l| l.starts_with("SECTION[id="))
        .unwrap_or(false)
}

/// Parse the LLM-mode export back into an `IrDocument`. Always
/// produces at least one section (matching the empty-input behaviour
/// of `from_markdown`).
pub fn from_llm_markdown(src: &str) -> Result<IrDocument, IrError> {
    let mut doc = IrDocument::default();
    // Same 7-slot heading/styles/charShape table the GFM importer
    // builds, so heading round-trip is consistent across both MD
    // import paths and the HWPX writer's surgical rewriter has the
    // shapes to splice into the bundled skeleton.
    super::style_synth::populate_heading_doc_info(&mut doc);

    let mut sections: Vec<Section> = Vec::new();
    let mut current = Section::default();
    let mut state = State::Idle;
    // True once we've crossed `<!-- hwp-transpiler: assets -->`. From
    // that point on, body-level records (FIGURE / TABLE / PARAGRAPH)
    // stop landing into the section and ASSET / DATA pairs flow into
    // `doc.bin_data` instead.
    let mut in_assets = false;
    let mut pending_asset: Option<PendingAsset> = None;
    // When a `SECTION_BYTES[id=section-N,…]` record is seen, the
    // following `DATA:` line decodes into `sections[N].stream_bytes`.
    // Mutually exclusive with `pending_asset` — both branches read
    // the next DATA line from the same handler.
    let mut pending_section_idx: Option<usize> = None;

    for line in src.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed == "<!-- hwp-transpiler: assets -->" {
            // Flush any in-flight body state and switch to footer
            // mode. Subsequent SECTION[ markers (defensive) are
            // ignored — assets aren't sectioned.
            flush_state(&mut state, &mut current);
            sections.push(std::mem::take(&mut current));
            state = State::Idle;
            in_assets = true;
            continue;
        }

        if in_assets {
            if trimmed.starts_with("ASSET[") {
                let attrs = parse_attrs(trimmed);
                let bin_id = attrs.get_int("bin_id").map(|n| n.clamp(0, u16::MAX as i64) as u16);
                let source_id = attrs
                    .0
                    .get("source_id")
                    .cloned()
                    .or_else(|| {
                        // Fall back to a sensible default name when the exporter
                        // didn't stamp one. HWPX convention.
                        bin_id.map(|n| format!("image{n}.png"))
                    })
                    .unwrap_or_default();
                pending_asset = Some(PendingAsset { bin_id, source_id });
                pending_section_idx = None;
                continue;
            }
            if trimmed.starts_with("SECTION_BYTES[") {
                let attrs = parse_attrs(trimmed);
                // id="section-N" → N
                pending_section_idx = attrs
                    .0
                    .get("id")
                    .and_then(|s| s.strip_prefix("section-"))
                    .and_then(|s| s.parse::<usize>().ok());
                pending_asset = None;
                continue;
            }
            if let Some(uri) = trimmed.strip_prefix("DATA: ") {
                if let Some(p) = pending_asset.take() {
                    if let Some(entry) = decode_data_uri_to_binary_entry(uri.trim(), &p.source_id) {
                        doc.bin_data.push(entry);
                    }
                } else if let Some(idx) = pending_section_idx.take() {
                    // `data:application/octet-stream;base64,…` → raw
                    // bytes back into the matching section's
                    // verbatim cache. Writer's existing
                    // stream_bytes-first emit handles re-using them.
                    if let Some(bytes) = decode_octet_stream_data_uri(uri.trim()) {
                        if let Some(section) = sections.get_mut(idx) {
                            section.stream_bytes = Some(bytes);
                        }
                    }
                }
                continue;
            }
            // Other lines in footer (blank, comments) — ignore.
            continue;
        }

        if trimmed.starts_with("SECTION[") {
            // Flush prior pending state into the current section,
            // then push it (but only if it actually carries
            // anything — the first SECTION marker arrives before
            // any content, so an empty `current` here is the
            // leading buffer and would shift `SECTION_BYTES
            // [id=section-N]` indexing off by one).
            flush_state(&mut state, &mut current);
            if !current.paragraphs.is_empty() || current.stream_bytes.is_some() {
                sections.push(std::mem::take(&mut current));
            } else {
                current = Section::default();
            }
            state = State::Idle;
            continue;
        }

        if trimmed.starts_with("PARAGRAPH[") {
            flush_state(&mut state, &mut current);
            // Heading level can be supplied two ways:
            //   * Explicit `level=N` attribute on the PARAGRAPH
            //     record — preferred, requires exporter cooperation.
            //   * Markdown-style `# `…`###### ` text prefix on the
            //     following TEXT line — fallback for already-emitted
            //     docs that don't carry the attr.
            let attrs = parse_attrs(trimmed);
            let level = attrs.get_int("level").map(|n| n.clamp(0, 6) as u8);
            state = State::ExpectingParagraphText { explicit_level: level };
            continue;
        }

        if trimmed.starts_with("TABLE[") {
            flush_state(&mut state, &mut current);
            state = State::InTable(LlmTableBuilder::default());
            continue;
        }

        if trimmed.starts_with("END TABLE") {
            if let State::InTable(builder) = std::mem::replace(&mut state, State::Idle) {
                let table = builder.finish();
                let mut wrapper = Paragraph::default();
                wrapper.text = "\u{FFFC}".into();
                wrapper.controls.push(Control {
                    kind: ControlKind::Table(table),
                    caption_text: None,
                });
                current.paragraphs.push(wrapper);
            }
            continue;
        }

        if trimmed.starts_with("FIGURE[") {
            flush_state(&mut state, &mut current);
            let attrs = parse_attrs(trimmed);
            let bin_id = attrs.get_int("bin_id").map(|n| n.clamp(0, u16::MAX as i64) as u16);
            let width_mm = attrs.get_int("width_mm").unwrap_or(0).max(0) as u32;
            let height_mm = attrs.get_int("height_mm").unwrap_or(0).max(0) as u32;
            // mm → HWPUNIT (7200 / 25.4 = ~283.46). Multiply via
            // f64 then round to keep the conversion symmetric with
            // the export side's `hwpunit_to_mm`.
            let width_hwpu = ((width_mm as f64) * 7200.0 / 25.4).round() as u32;
            let height_hwpu = ((height_mm as f64) * 7200.0 / 25.4).round() as u32;
            let mut wrapper = Paragraph::default();
            wrapper.text = "\u{FFFC}".into();
            wrapper.controls.push(Control {
                kind: ControlKind::Picture(PictureControl {
                    bin_id: bin_id.unwrap_or(0),
                    width_hwpu,
                    height_hwpu,
                }),
                caption_text: None,
            });
            current.paragraphs.push(wrapper);
            continue;
        }

        if trimmed.starts_with("CELL[") {
            let attrs = parse_attrs(trimmed);
            let row = attrs.get_int("row").unwrap_or(0) as u16;
            let col = attrs.get_int("col").unwrap_or(0) as u16;
            let row_span = attrs.get_int("rowspan").unwrap_or(1).max(1) as u16;
            let col_span = attrs.get_int("colspan").unwrap_or(1).max(1) as u16;
            if let State::InTable(builder) = &mut state {
                builder.flush_pending();
                builder.pending = Some(PendingCell { row, col, row_span, col_span });
            }
            continue;
        }

        if let Some(text) = strip_text_prefix(trimmed) {
            match &mut state {
                State::ExpectingParagraphText { explicit_level } => {
                    let (level, body) = resolve_heading(*explicit_level, text);
                    current.paragraphs.push(make_paragraph(level, body));
                    state = State::Idle;
                }
                State::InTable(builder) => {
                    builder.set_pending_text(text.to_string());
                }
                State::Idle => {
                    // Bare TEXT without a preceding PARAGRAPH marker
                    // — treat as a body paragraph so prose isn't
                    // dropped on the floor.
                    current.paragraphs.push(make_paragraph(0, text.to_string()));
                }
            }
            continue;
        }
        // FIGURE, CAPTION, EQUATION, BREAK… anything else falls
        // through silently for now.
    }

    flush_state(&mut state, &mut current);
    if !in_assets {
        // The assets-footer transition already pushed the body
        // section. A second push here would land an empty trailing
        // section.
        sections.push(current);
    }
    let _ = pending_asset; // drop in case ASSET[…] had no following DATA

    // Drop empty leading sections from the SECTION-flush boundary —
    // the loop pushes one Section per SECTION marker, so the first
    // entry is always the empty pre-section buffer.
    if sections.first().is_some_and(|s| s.paragraphs.is_empty())
        && sections.len() > 1
    {
        sections.remove(0);
    }
    if sections.is_empty() {
        sections.push(Section::default());
    }
    doc.sections = sections;
    Ok(doc)
}

enum State {
    Idle,
    /// `explicit_level` is `Some(N)` when the PARAGRAPH record
    /// carried `level=N`. The TEXT branch falls back to a `# `-
    /// prefix scan when this is `None`.
    ExpectingParagraphText {
        explicit_level: Option<u8>,
    },
    InTable(LlmTableBuilder),
}

/// Held between an `ASSET[…]` line and its following `DATA: …`
/// line so the data URI decode happens with the asset's id /
/// bin_id metadata in hand.
struct PendingAsset {
    /// Reserved for future use — `BinaryEntry.id` is currently the
    /// only field needed downstream, but keeping `bin_id` here means
    /// any later cross-validation (PictureControl.bin_id matching
    /// ASSET.bin_id) doesn't need a re-parse.
    #[allow(dead_code)]
    bin_id: Option<u16>,
    source_id: String,
}

#[derive(Default)]
struct LlmTableBuilder {
    rows: u16,
    cols: u16,
    cells: Vec<TableCell>,
    pending: Option<PendingCell>,
}

struct PendingCell {
    row: u16,
    col: u16,
    row_span: u16,
    col_span: u16,
}

impl LlmTableBuilder {
    fn set_pending_text(&mut self, text: String) {
        if let Some(p) = self.pending.take() {
            self.push_cell(p, text);
        }
    }
    fn flush_pending(&mut self) {
        if let Some(p) = self.pending.take() {
            self.push_cell(p, String::new());
        }
    }
    fn push_cell(&mut self, p: PendingCell, text: String) {
        let mut para = Paragraph::default();
        para.text = text;
        self.cells.push(TableCell {
            row: p.row,
            col: p.col,
            row_span: p.row_span,
            col_span: p.col_span,
            para_count: 1,
            // Bundled skeleton's borderFill id=1 carries solid 0.12 mm
            // black borders on all four sides — references it so cell
            // outlines are visible in HWPX viewers (id=0 is the
            // no-border default).
            border_fill_id: 1,
            paragraphs: vec![para],
            ..TableCell::default()
        });
        self.rows = self.rows.max(p.row + p.row_span);
        self.cols = self.cols.max(p.col + p.col_span);
    }
    fn finish(mut self) -> TableControl {
        self.flush_pending();
        // Distribute the default A4 text-region width evenly across
        // columns so HWPX viewers don't collapse 0-width cells.
        // Heights default to a single text-row.
        super::cell_sizes::apply_defaults(&mut self.cells, self.cols);
        let row_cell_counts = vec![self.cols; self.rows as usize];
        TableControl {
            rows: self.rows,
            cols: self.cols,
            row_cell_counts,
            cells: self.cells,
            ..TableControl::default()
        }
    }
}

fn flush_state(state: &mut State, section: &mut Section) {
    match std::mem::replace(state, State::Idle) {
        State::Idle | State::ExpectingParagraphText { .. } => {}
        State::InTable(builder) => {
            // Mid-table flush at section boundary — emit what we
            // have so cells aren't lost.
            let table = builder.finish();
            let mut wrapper = Paragraph::default();
            wrapper.text = "\u{FFFC}".into();
            wrapper.controls.push(Control {
                kind: ControlKind::Table(table),
                caption_text: None,
            });
            section.paragraphs.push(wrapper);
        }
    }
}

/// Pull the comma-separated `key=value` attribute set out of a
/// `KIND[…]` record line. Tolerates whitespace and stops scanning at
/// the first `]`. Values can't contain `,` or `]` (matches the
/// exporter's emit guarantees).
fn parse_attrs(line: &str) -> AttrMap {
    let mut map = HashMap::new();
    let Some(open) = line.find('[') else {
        return AttrMap(map);
    };
    let rest = &line[open + 1..];
    let body = rest.split(']').next().unwrap_or("");
    for piece in body.split(',') {
        let piece = piece.trim();
        if let Some((k, v)) = piece.split_once('=') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    AttrMap(map)
}

struct AttrMap(HashMap<String, String>);

impl AttrMap {
    fn get_int(&self, key: &str) -> Option<i64> {
        self.0.get(key).and_then(|v| v.parse::<i64>().ok())
    }
}

/// Build a body paragraph (`level=0`) or a heading paragraph
/// (`level=1..=6`). Heading paragraphs reference the matching
/// `style_id` / `para_shape_id` / `char_shape_id` so HWPX viewers
/// render with the heading style/font from the synthesised
/// DocInfo table.
fn make_paragraph(level: u8, text: String) -> Paragraph {
    let mut p = Paragraph::default();
    p.text = text;
    p.header = ParagraphHeader {
        style_id: level,
        para_shape_id: level as u16,
        ..ParagraphHeader::default()
    };
    if level > 0 {
        p.char_shape_runs.push(CharShapeRun {
            start: 0,
            char_shape_id: level as u32,
        });
    }
    p
}

/// Decide the heading level for a TEXT line. Explicit `level=N`
/// from the PARAGRAPH record wins; otherwise check for a Markdown
/// `# ` … `###### ` prefix on the text and strip it. Returns the
/// `(level, body)` pair after any prefix removal.
fn resolve_heading(explicit: Option<u8>, text: &str) -> (u8, String) {
    if let Some(level) = explicit {
        if level > 0 {
            return (level, text.to_string());
        }
    }
    if let Some((level, rest)) = strip_atx_heading_prefix(text) {
        return (level, rest.to_string());
    }
    (explicit.unwrap_or(0), text.to_string())
}

/// `"## Foo"` → `Some((2, "Foo"))`. Returns `None` when the line
/// isn't an ATX heading.
fn strip_atx_heading_prefix(text: &str) -> Option<(u8, &str)> {
    let mut hash_count = 0u8;
    for c in text.chars() {
        if c == '#' {
            hash_count += 1;
            if hash_count > 6 {
                return None;
            }
        } else {
            break;
        }
    }
    if hash_count == 0 {
        return None;
    }
    let after_hashes = &text[hash_count as usize..];
    let body = after_hashes.strip_prefix(' ')?;
    Some((hash_count, body))
}

/// Strip `TEXT: ` or `TEXT[…]: ` prefix; returns `None` for non-TEXT
/// lines.
fn strip_text_prefix(line: &str) -> Option<&str> {
    if let Some(rest) = line.strip_prefix("TEXT: ") {
        return Some(rest);
    }
    let rest = line.strip_prefix("TEXT[")?;
    let close = rest.find("]: ")?;
    Some(&rest[close + 3..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_llm_format_by_leading_sigil() {
        assert!(looks_like_llm_format("SECTION[id=sec-0]\n"));
        assert!(looks_like_llm_format("\n\nSECTION[id=sec-0]\n"));
        assert!(!looks_like_llm_format("# Heading\n"));
        assert!(!looks_like_llm_format("Hello body."));
        assert!(!looks_like_llm_format(""));
    }

    #[test]
    fn empty_section_yields_one_empty_section() {
        let doc = from_llm_markdown("SECTION[id=sec-0]\n").expect("parse");
        assert_eq!(doc.sections.len(), 1);
        assert!(doc.sections[0].paragraphs.is_empty());
    }

    #[test]
    fn paragraph_text_collected() {
        let src = "SECTION[id=sec-0]\n\nPARAGRAPH[id=par-s0-p0]\nTEXT: 안녕\n";
        let doc = from_llm_markdown(src).expect("parse");
        assert_eq!(doc.sections[0].paragraphs.len(), 1);
        assert_eq!(doc.sections[0].paragraphs[0].text, "안녕");
    }

    #[test]
    fn simple_2x2_table_lands_with_correct_dims() {
        let src = "\
SECTION[id=sec-0]

TABLE[id=t,rows=2,cols=2]
CELL[id=c00,row=0,col=0,rowspan=1,colspan=1]
TEXT[c00-p0]: A
CELL[id=c01,row=0,col=1,rowspan=1,colspan=1]
TEXT[c01-p0]: B
CELL[id=c10,row=1,col=0,rowspan=1,colspan=1]
TEXT[c10-p0]: 1
CELL[id=c11,row=1,col=1,rowspan=1,colspan=1]
TEXT[c11-p0]: 2
END TABLE[t]
";
        let doc = from_llm_markdown(src).expect("parse");
        let table = match &doc.sections[0].paragraphs[0].controls[0].kind {
            ControlKind::Table(t) => t,
            _ => panic!("expected table"),
        };
        assert_eq!(table.rows, 2);
        assert_eq!(table.cols, 2);
        assert_eq!(table.cells.len(), 4);
    }

    #[test]
    fn cell_rowspan_and_colspan_extend_table_dims() {
        let src = "\
SECTION[id=sec-0]

TABLE[id=t,rows=2,cols=3]
CELL[id=c,row=0,col=0,rowspan=2,colspan=3]
TEXT[c-p0]: merged
END TABLE[t]
";
        let doc = from_llm_markdown(src).expect("parse");
        let table = match &doc.sections[0].paragraphs[0].controls[0].kind {
            ControlKind::Table(t) => t,
            _ => panic!(),
        };
        assert_eq!(table.rows, 2);
        assert_eq!(table.cols, 3);
        assert_eq!(table.cells[0].row_span, 2);
        assert_eq!(table.cells[0].col_span, 3);
        assert_eq!(table.cells[0].paragraphs[0].text, "merged");
    }

    #[test]
    fn paragraph_then_table_then_paragraph_preserves_order() {
        let src = "\
SECTION[id=sec-0]

PARAGRAPH[id=p0]
TEXT: 첫 단락

TABLE[id=t,rows=1,cols=1]
CELL[id=c,row=0,col=0,rowspan=1,colspan=1]
TEXT[c-p0]: 셀
END TABLE[t]

PARAGRAPH[id=p1]
TEXT: 마지막 단락
";
        let doc = from_llm_markdown(src).expect("parse");
        let ps = &doc.sections[0].paragraphs;
        assert_eq!(ps.len(), 3);
        assert_eq!(ps[0].text, "첫 단락");
        assert_eq!(ps[1].text, "\u{FFFC}");
        assert!(matches!(ps[1].controls[0].kind, ControlKind::Table(_)));
        assert_eq!(ps[2].text, "마지막 단락");
    }

    #[test]
    fn parse_attrs_handles_whitespace_and_extra_keys() {
        let attrs = parse_attrs("CELL[ row = 3, col=4, rowspan = 2, colspan=5, role=header ]");
        assert_eq!(attrs.get_int("row"), Some(3));
        assert_eq!(attrs.get_int("col"), Some(4));
        assert_eq!(attrs.get_int("rowspan"), Some(2));
        assert_eq!(attrs.get_int("colspan"), Some(5));
        assert_eq!(attrs.0.get("role").map(String::as_str), Some("header"));
    }

    #[test]
    fn strip_text_prefix_handles_both_forms() {
        assert_eq!(strip_text_prefix("TEXT: hello"), Some("hello"));
        assert_eq!(strip_text_prefix("TEXT[some-id]: hello"), Some("hello"));
        assert_eq!(strip_text_prefix("not text"), None);
    }
}
