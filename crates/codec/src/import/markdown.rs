//! Markdown → [`IrDocument`] importer.
//!
//! Supported blocks:
//!   * ATX-style headings (`#` … `######`).
//!   * Plain paragraphs (soft breaks → spaces, hard breaks → `\n`).
//!   * GFM pipe tables (uniform grid, `col_span = row_span = 1` per
//!     cell, no merge). Each table emits a wrapper `Paragraph`
//!     carrying a single `ControlKind::Table` control with text set
//!     to the IR's `\u{FFFC}` object-replacement marker, matching
//!     the convention HWP / HWPX writers expect.
//!
//! Inline emphasis (`**bold**`, `*italic*`) is decoded via
//! pulldown-cmark events but flattened to plain text in the IR for
//! now — typed `CharShapeRun` decoration is the next iteration.
//!
//! Lists, code blocks, links, images, equations, and the domain-
//! specific hint annotations the exporter emits (cell merge spans,
//! role tags, …) are not yet handled. They land as text in the
//! surrounding paragraph rather than throwing — graceful degradation
//! while we iterate.
//!
//! Output shape — every imported document is one `Section`,
//! paragraphs collected in source order, with synthesised
//! `ParaShape`s 0..=6 (index 0 = body, 1..=6 = heading levels) so
//! the existing exporter / renderer / HWPX writer can read back the
//! heading bits via `ParaShape::heading_level()`.

use hwp_transpiler_core::ir::{
    CharShapeRun, Control, ControlKind, IrDocument, IrError, Paragraph, ParagraphHeader, Section,
    TableCell, TableControl,
};
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// Parse a UTF-8 Markdown string into an [`IrDocument`].
///
/// Auto-dispatches: if the input is recognisably the LLM emit format
/// (leading `SECTION[id=…]` line), routes to
/// [`super::markdown_llm::from_llm_markdown`]; otherwise parses as
/// CommonMark with GFM tables.
///
/// Always succeeds for valid UTF-8 input — the `Result` shape mirrors
/// HWP / HWPX readers so callers can treat all import paths
/// uniformly.
pub fn from_markdown(src: &str) -> Result<IrDocument, IrError> {
    match detect_format(src) {
        FormatHint::Llm => super::markdown_llm::from_llm_markdown(src),
        FormatHint::Human => from_gfm_markdown(src),
        FormatHint::Unknown => {
            // No explicit header — fall back to sigil-sniffing the
            // LLM record format so docs from older exports still
            // dispatch correctly.
            if super::markdown_llm::looks_like_llm_format(src) {
                super::markdown_llm::from_llm_markdown(src)
            } else {
                from_gfm_markdown(src)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormatHint {
    Human,
    Llm,
    Unknown,
}

/// Read the leading `<!-- hwp-transpiler: format=… -->` HTML comment
/// stamped by the matching exporter. Tolerates blank lines before
/// the marker. Anything else returns `Unknown` so the caller can
/// fall back to content-based detection.
fn detect_format(src: &str) -> FormatHint {
    for line in src.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("<!-- hwp-transpiler: format=") {
            if let Some(value) = rest.strip_suffix("-->") {
                return match value.trim() {
                    "human" => FormatHint::Human,
                    "llm" => FormatHint::Llm,
                    _ => FormatHint::Unknown,
                };
            }
        }
        return FormatHint::Unknown;
    }
    FormatHint::Unknown
}

/// Plain GFM-Markdown branch of `from_markdown`. Exposed as its own
/// fn so the LLM dispatcher can opt in / out cleanly, and tests can
/// target one path without the auto-detect clouding which entrypoint
/// they exercised.
pub fn from_gfm_markdown(src: &str) -> Result<IrDocument, IrError> {
    let mut doc = IrDocument::default();
    super::style_synth::populate_heading_doc_info(&mut doc);

    let mut section = Section::default();
    let mut state = ParseState::Idle;

    for event in Parser::new_ext(src, Options::ENABLE_TABLES) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                flush(&mut state, &mut section);
                state = ParseState::Collecting {
                    level: heading_level_to_u8(level),
                    text: String::new(),
                };
            }
            Event::Start(Tag::Paragraph) => {
                flush(&mut state, &mut section);
                state = ParseState::Collecting {
                    level: 0, // body
                    text: String::new(),
                };
            }
            Event::Start(Tag::Table(alignments)) => {
                flush(&mut state, &mut section);
                state = ParseState::Table(TableBuilder::new(alignments.len() as u16));
            }
            Event::Start(Tag::TableHead) | Event::Start(Tag::TableRow) => {
                if let ParseState::Table(t) = &mut state {
                    t.current_col = 0;
                }
            }
            Event::Start(Tag::TableCell) => {
                if let ParseState::Table(t) = &mut state {
                    t.in_cell = true;
                    t.cell_text.clear();
                }
            }
            Event::End(TagEnd::TableCell) => {
                if let ParseState::Table(t) = &mut state {
                    t.finish_cell();
                }
            }
            Event::End(TagEnd::TableHead) | Event::End(TagEnd::TableRow) => {
                if let ParseState::Table(t) = &mut state {
                    t.current_row += 1;
                }
            }
            Event::End(TagEnd::Table) => {
                if let ParseState::Table(t) =
                    std::mem::replace(&mut state, ParseState::Idle)
                {
                    let mut wrapper = Paragraph::default();
                    wrapper.text = "\u{FFFC}".to_string();
                    wrapper.controls.push(Control {
                        kind: ControlKind::Table(t.finish()),
                        caption_text: None,
                    });
                    section.paragraphs.push(wrapper);
                }
            }
            Event::Text(t) | Event::Code(t) => match &mut state {
                ParseState::Collecting { text, .. } => text.push_str(&t),
                ParseState::Table(b) if b.in_cell => b.cell_text.push_str(&t),
                _ => {}
            },
            Event::SoftBreak => match &mut state {
                ParseState::Collecting { text, .. } => text.push(' '),
                ParseState::Table(b) if b.in_cell => b.cell_text.push(' '),
                _ => {}
            },
            Event::HardBreak => match &mut state {
                ParseState::Collecting { text, .. } => text.push('\n'),
                ParseState::Table(b) if b.in_cell => b.cell_text.push('\n'),
                _ => {}
            },
            Event::End(TagEnd::Heading(_)) | Event::End(TagEnd::Paragraph) => {
                flush(&mut state, &mut section);
            }
            // Inline emphasis (`**bold**`, `*italic*`) opens with
            // Start(Strong)/Start(Emphasis), encloses Text events,
            // closes with the matching End. First slice surfaces the
            // inner text without typed formatting — the wrapper
            // events fall through to the catch-all here and the
            // contained Text events flow to whichever buffer is
            // currently active.
            _ => {}
        }
    }
    // Final flush in case the source didn't end with a block close
    // (rare but possible with truncated input).
    flush(&mut state, &mut section);

    doc.sections.push(section);
    Ok(doc)
}

enum ParseState {
    Idle,
    /// `level = 0` → body paragraph; `1..=6` → heading. Both
    /// `style_id` and `para_shape_id` use this value on flush so the
    /// existing exporter (which keys off `style.name`) and any future
    /// `ParaShape::heading_level()` consumer agree.
    Collecting { level: u8, text: String },
    /// Inside a `<Tag::Table>` — rows / cells accumulate into
    /// `TableBuilder` until `End(Table)` flushes them as a single
    /// wrapper Paragraph carrying a `ControlKind::Table` control.
    Table(TableBuilder),
}

/// Accumulator for the current Markdown table. MD tables are uniform
/// grids (no merge), so cells always have `col_span = row_span = 1`
/// and every row carries exactly `cols` cells.
struct TableBuilder {
    cols: u16,
    current_row: u16,
    current_col: u16,
    cells: Vec<TableCell>,
    cell_text: String,
    in_cell: bool,
}

impl TableBuilder {
    fn new(cols: u16) -> Self {
        Self {
            cols,
            current_row: 0,
            current_col: 0,
            cells: Vec::new(),
            cell_text: String::new(),
            in_cell: false,
        }
    }

    fn finish_cell(&mut self) {
        if !self.in_cell {
            return;
        }
        let mut para = Paragraph::default();
        para.text = std::mem::take(&mut self.cell_text);
        let cell = TableCell {
            row: self.current_row,
            col: self.current_col,
            col_span: 1,
            row_span: 1,
            para_count: 1,
            // See LLM importer: id=1 in the bundled skeleton has
            // visible solid borders on all four sides.
            border_fill_id: 1,
            paragraphs: vec![para],
            ..TableCell::default()
        };
        self.cells.push(cell);
        self.current_col += 1;
        self.in_cell = false;
    }

    fn finish(mut self) -> TableControl {
        let rows = self.current_row;
        super::cell_sizes::apply_defaults(&mut self.cells, self.cols);
        let row_cell_counts = vec![self.cols; rows as usize];
        TableControl {
            rows,
            cols: self.cols,
            row_cell_counts,
            cells: self.cells,
            ..TableControl::default()
        }
    }
}

fn flush(state: &mut ParseState, section: &mut Section) {
    if let ParseState::Collecting { level, text } =
        std::mem::replace(state, ParseState::Idle)
    {
        if text.is_empty() {
            return;
        }
        let mut p = Paragraph::default();
        p.text = text;
        p.header = ParagraphHeader {
            style_id: level,
            para_shape_id: level as u16,
            ..ParagraphHeader::default()
        };
        // Heading paragraphs point at the matching heading CharShape
        // so HWPX viewers render them with the bigger / bolder font
        // synthesised in `synthesise_heading_char_shapes`. Body
        // paragraphs default to charPrIDRef=0, which the
        // section_writer emits when char_shape_runs is empty.
        if level > 0 {
            p.char_shape_runs.push(CharShapeRun {
                start: 0,
                char_shape_id: level as u32,
            });
        }
        section.paragraphs.push(p);
    }
}

fn heading_level_to_u8(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_one_empty_section() {
        let doc = from_markdown("").expect("parse");
        assert_eq!(doc.sections.len(), 1);
        assert!(doc.sections[0].paragraphs.is_empty());
    }

    #[test]
    fn plain_paragraph_collected_as_body() {
        let doc = from_markdown("Hello world").expect("parse");
        let p = &doc.sections[0].paragraphs[0];
        assert_eq!(p.text, "Hello world");
        assert_eq!(p.header.style_id, 0);
        assert_eq!(p.header.para_shape_id, 0);
    }

    #[test]
    fn atx_heading_assigns_level_one_style_and_shape() {
        let doc = from_markdown("# Title").expect("parse");
        assert_eq!(doc.sections[0].paragraphs.len(), 1);
        let p = &doc.sections[0].paragraphs[0];
        assert_eq!(p.text, "Title");
        assert_eq!(p.header.style_id, 1);
        assert_eq!(p.header.para_shape_id, 1);
    }

    #[test]
    fn heading_levels_assign_distinct_ids() {
        let src = "# H1\n\n## H2\n\n###### H6";
        let doc = from_markdown(src).expect("parse");
        let paras = &doc.sections[0].paragraphs;
        assert_eq!(paras.len(), 3);
        assert_eq!(paras[0].header.style_id, 1);
        assert_eq!(paras[1].header.style_id, 2);
        assert_eq!(paras[2].header.style_id, 6);
    }

    #[test]
    fn synthesised_styles_use_outline_naming_for_export_round_trip() {
        let doc = from_markdown("# X").expect("parse");
        let styles = &doc.doc_info.styles;
        assert_eq!(styles[1].name, "개요 1");
        assert_eq!(styles[6].name, "개요 6");
        assert_eq!(styles[1].english_name, "Outline 1");
    }

    #[test]
    fn heading_para_shape_table_carries_heading_level() {
        let doc = from_markdown("## Foo").expect("parse");
        let para_shapes = &doc.doc_info.para_shapes;
        assert!(para_shapes.len() >= 7, "expected synthesised 0..=6");
        assert_eq!(para_shapes[0].heading_level(), 0, "body has no level");
        assert_eq!(para_shapes[1].heading_level(), 1);
        assert_eq!(para_shapes[2].heading_level(), 2);
        assert_eq!(para_shapes[6].heading_level(), 6);
    }

    #[test]
    fn multiple_paragraphs_separated_by_blank_lines() {
        let src = "First.\n\nSecond.\n\nThird.";
        let doc = from_markdown(src).expect("parse");
        let texts: Vec<&str> = doc.sections[0]
            .paragraphs
            .iter()
            .map(|p| p.text.as_str())
            .collect();
        assert_eq!(texts, vec!["First.", "Second.", "Third."]);
    }

    #[test]
    fn soft_break_collapses_to_space() {
        // Two lines without blank between → one paragraph in CommonMark.
        let doc = from_markdown("first line\nsecond line").expect("parse");
        let p = &doc.sections[0].paragraphs[0];
        assert_eq!(p.text, "first line second line");
    }

    #[test]
    fn inline_emphasis_text_flattened_to_plain() {
        // Phase-1: bold / italic markers consumed but not yet
        // surfaced as char_shape_runs.
        let doc = from_markdown("a **bold** word").expect("parse");
        let p = &doc.sections[0].paragraphs[0];
        assert_eq!(p.text, "a bold word");
    }

    #[test]
    fn simple_table_creates_wrapper_paragraph_with_table_control() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |";
        let doc = from_markdown(md).expect("parse");
        let paras = &doc.sections[0].paragraphs;
        assert_eq!(paras.len(), 1, "table should land in one wrapper");
        let p = &paras[0];
        assert_eq!(p.text, "\u{FFFC}", "wrapper text marker present");
        assert_eq!(p.controls.len(), 1, "exactly one control");
        let table = match &p.controls[0].kind {
            ControlKind::Table(t) => t,
            _ => panic!("expected table control"),
        };
        assert_eq!(table.cols, 2);
        assert_eq!(table.rows, 3, "header + 2 body rows");
        assert_eq!(table.cells.len(), 6);
    }

    #[test]
    fn table_cell_positions_filled_in_row_major_order() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |";
        let doc = from_markdown(md).expect("parse");
        let table = match &doc.sections[0].paragraphs[0].controls[0].kind {
            ControlKind::Table(t) => t,
            _ => panic!(),
        };
        let positions: Vec<(u16, u16, &str)> = table
            .cells
            .iter()
            .map(|c| (c.row, c.col, c.paragraphs[0].text.as_str()))
            .collect();
        assert_eq!(
            positions,
            vec![
                (0, 0, "A"),
                (0, 1, "B"),
                (1, 0, "1"),
                (1, 1, "2"),
            ],
        );
    }

    #[test]
    fn table_cells_default_to_unit_spans() {
        let md = "| A |\n|---|\n| 1 |";
        let doc = from_markdown(md).expect("parse");
        let table = match &doc.sections[0].paragraphs[0].controls[0].kind {
            ControlKind::Table(t) => t,
            _ => panic!(),
        };
        for c in &table.cells {
            assert_eq!(c.col_span, 1);
            assert_eq!(c.row_span, 1);
        }
    }

    #[test]
    fn body_then_table_then_body_preserves_section_order() {
        let md = "Before.\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\nAfter.";
        let doc = from_markdown(md).expect("parse");
        let ps = &doc.sections[0].paragraphs;
        assert_eq!(ps.len(), 3);
        assert_eq!(ps[0].text, "Before.");
        assert!(matches!(
            ps[1].controls[0].kind,
            ControlKind::Table(_)
        ));
        assert_eq!(ps[2].text, "After.");
    }

    #[test]
    fn table_cell_with_inline_emphasis_flattens_to_text() {
        // First slice: bold/italic in cells emit plain text, same as
        // body paragraphs.
        let md = "| Plain | Styled |\n|---|---|\n| a | **bold** |";
        let doc = from_markdown(md).expect("parse");
        let table = match &doc.sections[0].paragraphs[0].controls[0].kind {
            ControlKind::Table(t) => t,
            _ => panic!(),
        };
        assert_eq!(table.cells[3].paragraphs[0].text, "bold");
    }

    #[test]
    fn heading_then_body_then_heading_round_trips_order() {
        let src = "# A\n\nbody-1\n\n## B\n\nbody-2";
        let doc = from_markdown(src).expect("parse");
        let ps = &doc.sections[0].paragraphs;
        assert_eq!(ps.len(), 4);
        assert_eq!(ps[0].text, "A");
        assert_eq!(ps[0].header.para_shape_id, 1);
        assert_eq!(ps[1].text, "body-1");
        assert_eq!(ps[1].header.para_shape_id, 0);
        assert_eq!(ps[2].text, "B");
        assert_eq!(ps[2].header.para_shape_id, 2);
        assert_eq!(ps[3].text, "body-2");
        assert_eq!(ps[3].header.para_shape_id, 0);
    }
}
