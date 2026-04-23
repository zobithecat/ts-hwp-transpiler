//! Browser-preview HTML emitter.
//!
//! Produces a Markdown-independent *preview* surface from `IrDocument`.
//! Where the codec's Markdown exporter trades structure for prose (complex
//! tables become bullet lists, rowspan/colspan are flattened with
//! annotations), the HTML output preserves the visual structure: real
//! `<table>` elements with `rowspan`/`colspan` attributes, real
//! `<figure>` / `<figcaption>` pairs, semantic headings from DocInfo
//! style names.
//!
//! Output is a fragment — typically an `<article class="hwp-preview">`
//! wrapping one `<section>` per HWP section — so the caller can embed
//! it inside any page shell and style it with its own CSS. No inline
//! font/colour reconstruction in v1; the goal is accurate structure,
//! and type-specific styling lands later.
//!
//! Not implemented yet:
//!   - page / column boundaries
//!   - equation rendering (passed through as text)
//!   - CharShape-level font / colour
//!   - box-as-heading decoding (handled in Markdown path only)

use hwp_transpiler_core::ir::{
    BinData, ControlKind, IrDocument, Paragraph, PictureControl, TableCell, TableControl,
};

/// Rendering knobs. Mirrors `codec::export::markdown::MdOptions`
/// deliberately so a CLI can feed the same options to both paths.
#[derive(Debug, Clone, Default)]
pub struct HtmlOptions {
    /// Relative URL prefix for `<img src="…">` — typically the sidecar
    /// asset directory. When `None`, `<img>` tags are omitted from
    /// figures (caption-only rendering).
    pub assets_path: Option<String>,
}

pub fn to_html(doc: &IrDocument) -> String {
    to_html_with(doc, &HtmlOptions::default())
}

pub fn to_html_with(doc: &IrDocument, opts: &HtmlOptions) -> String {
    let mut out = String::new();
    out.push_str("<article class=\"hwp-preview\">\n");
    for section in &doc.sections {
        out.push_str("<section>\n");
        for para in &section.paragraphs {
            emit_paragraph(doc, para, &mut out, opts);
        }
        out.push_str("</section>\n");
    }
    out.push_str("</article>\n");
    out
}

fn emit_paragraph(doc: &IrDocument, para: &Paragraph, out: &mut String, opts: &HtmlOptions) {
    let text = clean_text(&para.text);
    if !text.is_empty() {
        match heading_level(doc, para) {
            Some(level) => {
                let tag = format!("h{}", level.clamp(1, 6));
                out.push('<');
                out.push_str(&tag);
                out.push('>');
                out.push_str(&escape_html(&text));
                out.push_str("</");
                out.push_str(&tag);
                out.push_str(">\n");
            }
            None => {
                out.push_str("<p>");
                out.push_str(&escape_html(&text));
                out.push_str("</p>\n");
            }
        }
    }
    for c in &para.controls {
        match &c.kind {
            ControlKind::Table(t) => emit_table(doc, t, out, opts),
            ControlKind::Picture(p) => {
                emit_figure(doc, p, c.caption_text.as_deref(), out, opts)
            }
            _ => {}
        }
    }
}

fn emit_table(doc: &IrDocument, t: &TableControl, out: &mut String, opts: &HtmlOptions) {
    out.push_str("<table>\n");
    // Group cells by row — cells covered by rowspan from a higher row
    // are *absent* from `t.cells` (HWP doesn't store phantom entries),
    // which is exactly what HTML expects: a `<tr>` lists only the
    // cells whose top-left corner falls in that row.
    for r in 0..t.rows {
        out.push_str("  <tr>\n");
        let mut row_cells: Vec<&TableCell> =
            t.cells.iter().filter(|c| c.row == r).collect();
        row_cells.sort_by_key(|c| c.col);
        for cell in row_cells {
            emit_cell(doc, cell, out, opts);
        }
        out.push_str("  </tr>\n");
    }
    out.push_str("</table>\n");
}

fn emit_cell(doc: &IrDocument, cell: &TableCell, out: &mut String, opts: &HtmlOptions) {
    out.push_str("    <td");
    if cell.row_span > 1 {
        out.push_str(&format!(" rowspan=\"{}\"", cell.row_span));
    }
    if cell.col_span > 1 {
        out.push_str(&format!(" colspan=\"{}\"", cell.col_span));
    }
    out.push_str(">");
    for (i, p) in cell.paragraphs.iter().enumerate() {
        let text = clean_text(&p.text);
        if !text.is_empty() {
            if i > 0 {
                out.push_str("<br>");
            }
            out.push_str(&escape_html(&text));
        }
        for ctrl in &p.controls {
            match &ctrl.kind {
                ControlKind::Table(nested) => emit_table(doc, nested, out, opts),
                ControlKind::Picture(pic) => {
                    emit_figure(doc, pic, ctrl.caption_text.as_deref(), out, opts)
                }
                _ => {}
            }
        }
    }
    out.push_str("</td>\n");
}

fn emit_figure(
    doc: &IrDocument,
    pic: &PictureControl,
    caption_text: Option<&str>,
    out: &mut String,
    opts: &HtmlOptions,
) {
    out.push_str("<figure>\n");
    if let Some(prefix) = &opts.assets_path {
        let filename = format!(
            "BIN{:04}.{}",
            pic.bin_id,
            resolve_bin_extension(doc, pic.bin_id)
        );
        let w_mm = hwpunit_to_mm(pic.width_hwpu);
        let h_mm = hwpunit_to_mm(pic.height_hwpu);
        out.push_str(&format!(
            "  <img src=\"{}/{}\" style=\"width:{}mm;height:{}mm\">\n",
            escape_attr(prefix),
            escape_attr(&filename),
            w_mm,
            h_mm
        ));
    }
    if let Some(cap) = caption_text {
        let cleaned = clean_text(cap);
        let stripped = strip_caption_label_prefix(&cleaned).trim();
        if !stripped.is_empty() {
            out.push_str("  <figcaption>");
            out.push_str(&escape_html(stripped));
            out.push_str("</figcaption>\n");
        }
    }
    out.push_str("</figure>\n");
}

fn resolve_bin_extension(doc: &IrDocument, bin_id: u16) -> &str {
    doc.doc_info
        .bin_data
        .iter()
        .find(|bd: &&BinData| bd.bin_data_id == Some(bin_id))
        .and_then(|bd| bd.extension.as_deref())
        .unwrap_or("bin")
}

fn hwpunit_to_mm(hwpu: u32) -> u32 {
    ((hwpu as f64) * 25.4 / 7200.0).round() as u32
}

/// Inspect the paragraph's `Style` (via DocInfo lookup) for the
/// Hangul/English outline-heading markers Hancom uses by default. Other
/// heading-detection heuristics (box-as-heading, numeric-prefix
/// promotion) live in the Markdown path — preview keeps one signal
/// source so the output maps 1:1 to on-disk styles.
fn heading_level(doc: &IrDocument, para: &Paragraph) -> Option<u8> {
    let style = doc.doc_info.styles.get(para.header.style_id as usize)?;
    for prefix in ["개요 ", "Outline "] {
        for name in [&style.name, &style.english_name] {
            if let Some(rest) = name.strip_prefix(prefix) {
                if let Ok(n) = rest.trim().parse::<u8>() {
                    if (1..=6).contains(&n) {
                        return Some(n);
                    }
                }
            }
        }
    }
    if style.name == "차례 제목" || style.english_name == "TOC Heading" {
        return Some(1);
    }
    None
}

/// Preview-local text cleaner. Matches the Markdown exporter's FFFC /
/// NBSP / em-space drops and PUA circled-digit translation. Kept
/// duplicate here (not cross-crate imported) because the render crate
/// intentionally stays one-way-dependent on core only.
fn clean_text(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\u{FFFC}' | '\u{00AD}' => {}
            '\u{00A0}' | '\u{2003}' => out.push(' '),
            _ => out.push(translate_pua_bullet(c).unwrap_or(c)),
        }
    }
    let mut squeezed = String::with_capacity(out.len());
    let mut last_space = false;
    for c in out.chars() {
        if c == ' ' {
            if !last_space {
                squeezed.push(' ');
            }
            last_space = true;
        } else {
            squeezed.push(c);
            last_space = false;
        }
    }
    squeezed.trim().to_string()
}

fn translate_pua_bullet(c: char) -> Option<char> {
    let n = c as u32;
    if (0xF02B1..=0xF02C4).contains(&n) {
        return char::from_u32(0x2460 + (n - 0xF02B1));
    }
    if (0xF2B1..=0xF2C4).contains(&n) {
        return char::from_u32(0x2460 + (n - 0xF2B1));
    }
    None
}

/// Mirrors the Markdown exporter: after `clean_text` drops U+FFFC
/// from HWP's auto-numbering caption field, `"그림 . <title>"` is
/// left stranded. Strip it for preview too so `<figcaption>` reads
/// cleanly. Yellow/table/Figure variants covered symmetrically.
fn strip_caption_label_prefix(s: &str) -> &str {
    for prefix in ["그림 . ", "표 . ", "Figure . ", "Table . "] {
        if let Some(rest) = s.strip_prefix(prefix) {
            return rest;
        }
    }
    s
}

/// Minimal HTML escape for text nodes. `&` must run first to avoid
/// double-escaping entities we introduced.
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Stricter escape for attribute values (double quote context).
fn escape_attr(s: &str) -> String {
    escape_html(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hwp_transpiler_core::ir::{
        Control, ControlKind, IrDocument, Paragraph, ParagraphHeader, Section, Style,
        TableCell, TableControl,
    };

    fn make_doc(styles: Vec<Style>, paragraphs: Vec<Paragraph>) -> IrDocument {
        let mut doc = IrDocument::default();
        doc.doc_info.styles = styles;
        doc.sections.push(Section {
            paragraphs,
            ..Section::default()
        });
        doc
    }

    fn style(name: &str) -> Style {
        Style {
            name: name.into(),
            english_name: String::new(),
            properties: 0,
            next_style_id: 0,
            lang_id: 0,
            para_shape_id: 0,
            char_shape_id: 0,
        }
    }

    fn para(style_id: u8, text: &str) -> Paragraph {
        Paragraph {
            header: ParagraphHeader { style_id, ..ParagraphHeader::default() },
            text: text.into(),
            ..Paragraph::default()
        }
    }

    fn cell(col: u16, row: u16, text: &str) -> TableCell {
        TableCell {
            col,
            row,
            col_span: 1,
            row_span: 1,
            paragraphs: vec![para(0, text)],
            ..TableCell::default()
        }
    }

    #[test]
    fn basic_paragraph_renders_as_p() {
        let doc = make_doc(vec![style("본문")], vec![para(0, "안녕하세요")]);
        let html = to_html(&doc);
        assert!(html.contains("<p>안녕하세요</p>"), "got: {html}");
    }

    #[test]
    fn outline_style_becomes_heading() {
        let doc = make_doc(
            vec![style("본문"), style("개요 2")],
            vec![para(1, "2장 제목")],
        );
        let html = to_html(&doc);
        assert!(html.contains("<h2>2장 제목</h2>"), "got: {html}");
    }

    #[test]
    fn html_special_chars_are_escaped() {
        let doc = make_doc(
            vec![style("본문")],
            vec![para(0, "A <b> & \"quoted\" O'Brien")],
        );
        let html = to_html(&doc);
        assert!(html.contains("A &lt;b&gt; &amp; &quot;quoted&quot; O&#39;Brien"));
        assert!(!html.contains("<b>"), "no raw tag leak");
    }

    #[test]
    fn simple_table_renders_with_tr_td() {
        let t = TableControl {
            rows: 2, cols: 2,
            row_cell_counts: vec![2, 2],
            cells: vec![
                cell(0, 0, "a"),
                cell(1, 0, "b"),
                cell(0, 1, "c"),
                cell(1, 1, "d"),
            ],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![Paragraph {
            controls: vec![Control { kind: ControlKind::Table(t), ..Default::default() }],
            ..Paragraph::default()
        }]);
        let html = to_html(&doc);
        assert!(html.contains("<table>"));
        assert!(html.contains("<tr>"));
        assert!(html.contains("<td>a</td>"));
        assert!(html.contains("<td>d</td>"));
    }

    #[test]
    fn rowspan_and_colspan_attrs_emit() {
        // A 2×3 table where row 0 has one cell with colspan=3, row 1
        // has one rowspan=1 + two plain cells.
        let t = TableControl {
            rows: 2, cols: 3, row_cell_counts: vec![1, 3],
            cells: vec![
                TableCell {
                    col: 0, row: 0, col_span: 3, row_span: 1,
                    paragraphs: vec![para(0, "merged header")],
                    ..TableCell::default()
                },
                cell(0, 1, "x"),
                cell(1, 1, "y"),
                cell(2, 1, "z"),
            ],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![Paragraph {
            controls: vec![Control { kind: ControlKind::Table(t), ..Default::default() }],
            ..Paragraph::default()
        }]);
        let html = to_html(&doc);
        assert!(
            html.contains("<td colspan=\"3\">merged header</td>"),
            "got: {html}"
        );
        assert!(html.contains("<td>x</td>"));
    }

    #[test]
    fn vertical_merge_adds_rowspan() {
        let t = TableControl {
            rows: 2, cols: 2, row_cell_counts: vec![2, 1],
            cells: vec![
                TableCell {
                    col: 0, row: 0, col_span: 1, row_span: 2,
                    paragraphs: vec![para(0, "vmerge")],
                    ..TableCell::default()
                },
                cell(1, 0, "a"),
                cell(1, 1, "b"),
            ],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![Paragraph {
            controls: vec![Control { kind: ControlKind::Table(t), ..Default::default() }],
            ..Paragraph::default()
        }]);
        let html = to_html(&doc);
        assert!(html.contains("<td rowspan=\"2\">vmerge</td>"));
    }

    #[test]
    fn figure_with_caption_and_assets_renders_img_and_figcaption() {
        use hwp_transpiler_core::ir::{BinData, PictureControl};
        let mut doc = IrDocument::default();
        doc.doc_info.bin_data.push(BinData {
            bin_data_id: Some(1),
            extension: Some("png".into()),
            ..BinData::default()
        });
        doc.sections.push(Section {
            paragraphs: vec![Paragraph {
                controls: vec![Control {
                    kind: ControlKind::Picture(PictureControl {
                        bin_id: 1,
                        width_hwpu: 7200,
                        height_hwpu: 3600,
                    }),
                    caption_text: Some("그림 \u{FFFC}. 시스템 도식".into()),
                }],
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        let html = to_html_with(
            &doc,
            &HtmlOptions { assets_path: Some("x.assets".into()) },
        );
        assert!(html.contains("<figure>"));
        assert!(
            html.contains("<img src=\"x.assets/BIN0001.png\" style=\"width:25mm;height:13mm\">"),
            "got: {html}"
        );
        assert!(html.contains("<figcaption>시스템 도식</figcaption>"));
    }

    #[test]
    fn figure_without_assets_renders_caption_only() {
        use hwp_transpiler_core::ir::PictureControl;
        let mut doc = IrDocument::default();
        doc.sections.push(Section {
            paragraphs: vec![Paragraph {
                controls: vec![Control {
                    kind: ControlKind::Picture(PictureControl {
                        bin_id: 1,
                        width_hwpu: 0,
                        height_hwpu: 0,
                    }),
                    caption_text: Some("그림 \u{FFFC}. 설명".into()),
                }],
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        let html = to_html(&doc);
        assert!(!html.contains("<img"));
        assert!(html.contains("<figcaption>설명</figcaption>"));
    }

    #[test]
    fn fffc_and_pua_bullets_cleaned_in_output() {
        let doc = make_doc(
            vec![style("본문")],
            vec![para(0, "\u{F2B1} 첫\u{FFFC} 째")],
        );
        let html = to_html(&doc);
        assert!(html.contains("① 첫 째"), "got: {html}");
        assert!(!html.contains('\u{FFFC}'));
    }
}
