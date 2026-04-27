//! Markdown → [`IrDocument`] importer.
//!
//! Phase-1 scope: ATX-style headings (`#` … `######`) and plain
//! paragraphs. Inline emphasis (`**bold**`, `*italic*`) is decoded
//! via pulldown-cmark events but flattened to plain text in the IR
//! for now — typed `CharShapeRun` decoration is the next iteration.
//!
//! Tables, lists, code blocks, links, images, equations, and the
//! domain-specific hint annotations the exporter emits (cell merge
//! spans, role tags, …) are not yet handled. They land as text in
//! the surrounding paragraph rather than throwing — graceful
//! degradation while we iterate.
//!
//! Output shape — every imported document is one `Section`,
//! paragraphs collected in source order, with synthesised
//! `ParaShape`s 0..=6 (index 0 = body, 1..=6 = heading levels) so
//! the existing exporter / renderer / HWPX writer can read back the
//! heading bits via `ParaShape::heading_level()`.

use hwp_transpiler_core::ir::{
    IrDocument, IrError, Paragraph, ParagraphHeader, ParaShape, Section, Style,
};
use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

/// Parse a UTF-8 Markdown string into an [`IrDocument`].
///
/// One Section, no DocInfo enrichment beyond the synthesised
/// ParaShape table. Always succeeds for valid UTF-8 input — the
/// `Result` shape mirrors HWP / HWPX readers so callers can treat
/// all import paths uniformly.
pub fn from_markdown(src: &str) -> Result<IrDocument, IrError> {
    let mut doc = IrDocument::default();
    doc.doc_info.para_shapes = synthesise_heading_para_shapes();
    doc.doc_info.styles = synthesise_heading_styles();

    let mut section = Section::default();
    let mut state = ParseState::Idle;

    for event in Parser::new(src) {
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
            Event::Text(t) | Event::Code(t) => {
                if let ParseState::Collecting { text, .. } = &mut state {
                    text.push_str(&t);
                }
            }
            Event::SoftBreak => {
                if let ParseState::Collecting { text, .. } = &mut state {
                    text.push(' ');
                }
            }
            Event::HardBreak => {
                if let ParseState::Collecting { text, .. } = &mut state {
                    text.push('\n');
                }
            }
            // (other branches unchanged)
            Event::End(TagEnd::Heading(_)) | Event::End(TagEnd::Paragraph) => {
                flush(&mut state, &mut section);
            }
            // Inline emphasis is observed via Start(Emphasis) /
            // End(Emphasis), with the wrapped Text events between.
            // First slice surfaces the inner text without typed
            // formatting — collection just keeps writing into the
            // current paragraph's buffer through the wrapper events.
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

/// `[default body, "개요 1", "개요 2", …, "개요 6"]` so the existing
/// human-Markdown exporter's `heading_level` lookup (which checks
/// `Style::name` for the `"개요 N"` prefix) re-classifies headings
/// after a round-trip. English fallback `"Outline N"` stays in
/// `english_name`.
fn synthesise_heading_styles() -> Vec<Style> {
    let mut styles = vec![Style::default()]; // body slot
    for level in 1u8..=6 {
        styles.push(Style {
            name: format!("개요 {level}"),
            english_name: format!("Outline {level}"),
            properties: 0,
            next_style_id: 0,
            lang_id: 0,
            para_shape_id: level as u16,
            char_shape_id: 0,
        });
    }
    styles
}

/// Build `[default, h1, h2, h3, h4, h5, h6]` so paragraph headers can
/// reference index 0 for body / 1..=6 for headings, matching
/// `ParaShape::heading_level()`'s encoding (level in bits 24..=26).
fn synthesise_heading_para_shapes() -> Vec<ParaShape> {
    let mut shapes = vec![ParaShape::default()];
    for level in 1u8..=6 {
        let mut p = ParaShape::default();
        p.attribute = (level as u32) << 24;
        shapes.push(p);
    }
    shapes
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
