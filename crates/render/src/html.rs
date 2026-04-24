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
//!   - column boundaries (page boundaries now covered by `emit_pages`)
//!   - equation rendering (passed through as text)
//!   - box-as-heading decoding (handled in Markdown path only)

use hwp_transpiler_core::ir::{
    BinData, CharShape, CharShapeRun, ControlKind, FontFaces, IrDocument, Paragraph,
    PictureControl, Section, SectionProperties, TableCell, TableControl,
};

/// Rendering knobs. Mirrors `codec::export::markdown::MdOptions`
/// deliberately so a CLI can feed the same options to both paths.
#[derive(Debug, Clone, Default)]
pub struct HtmlOptions {
    /// Relative URL prefix for `<img src="…">` — typically the sidecar
    /// asset directory. When `None`, `<img>` tags are omitted from
    /// figures (caption-only rendering).
    pub assets_path: Option<String>,
    /// Wrap each `CharShapeRun` in inline HTML: `<strong>`, `<em>`, `<s>`
    /// for bold/italic/strike, plus `<span style="color:…;font-size:…pt;
    /// font-family:…">` when the referenced `CharShape` deviates from
    /// HWP's defaults (black, 10pt, hangul slot 0). Runs on the document
    /// default pass through as plain text so the output stays readable
    /// when nothing is emphasised. Mirrors `MdOptions.emit_styles`.
    pub emit_styles: bool,
    /// Wrap each HWP section in `<section class="hwp-page" style="…">`
    /// carrying the page width/height and margins (HWPUNIT → mm). The
    /// caller styles the class with CSS to get a print-accurate preview
    /// shell. When off, sections render as plain unstyled `<section>`
    /// (the previous default behaviour).
    pub emit_pages: bool,
}

pub fn to_html(doc: &IrDocument) -> String {
    to_html_with(doc, &HtmlOptions::default())
}

pub fn to_html_with(doc: &IrDocument, opts: &HtmlOptions) -> String {
    let mut out = String::new();
    out.push_str("<article class=\"hwp-preview\">\n");
    for section in &doc.sections {
        emit_section_open(section, &mut out, opts);
        for para in &section.paragraphs {
            emit_paragraph(doc, para, &mut out, opts);
        }
        out.push_str("</section>\n");
    }
    out.push_str("</article>\n");
    if opts.emit_styles {
        out = hoist_inline_styles(&out);
    }
    out
}

/// Promote repeated `<span style="…">` declarations into a
/// document-scoped `<style>` block. A CharShape run's inline style
/// string often repeats hundreds of times in a full document (one
/// styled body run per paragraph), so leaving every occurrence inline
/// bloats the DOM without adding information. Styles that appear
/// **twice or more** get a short class name (`s0`, `s1`, …) scoped
/// under `.hwp-preview` so the rules can't bleed into a host page;
/// one-off styles stay inline since a class wouldn't save any bytes.
///
/// Output layout: the `<style>` block is prepended to the preview
/// fragment so the first thing the caller serialises is the stylesheet
/// followed by the `<article>`. When no style qualifies for hoisting
/// (document has no styled runs, or every run has a unique shape),
/// returns the input unchanged.
fn hoist_inline_styles(html: &str) -> String {
    use std::collections::HashMap;

    // Walk the HTML counting occurrences of each inline span style.
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let needle = r#"<span style=""#;
    let mut cursor = 0;
    while let Some(rel) = html[cursor..].find(needle) {
        let start = cursor + rel + needle.len();
        if let Some(end_rel) = html[start..].find('"') {
            let style = &html[start..start + end_rel];
            *counts.entry(style).or_insert(0) += 1;
            cursor = start + end_rel + 1;
        } else {
            break;
        }
    }

    // Assign class names to styles that repeat. Most-common-first so
    // the CSS order reflects usage; class names stay short (`s0` …)
    // since they're opaque anyway.
    let mut sorted: Vec<(&str, usize)> = counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let mut class_map: Vec<(String, String)> = Vec::new();
    for (i, (style, count)) in sorted.iter().enumerate() {
        if *count >= 2 {
            class_map.push((style.to_string(), format!("s{i}")));
        }
    }

    if class_map.is_empty() {
        return html.to_string();
    }

    // Build the <style> block. Scope under `.hwp-preview` so host
    // pages that embed the fragment don't inherit `s0` etc. globally.
    let mut css = String::from("<style>\n");
    for (style, cls) in &class_map {
        css.push_str(&format!(".hwp-preview .{cls} {{ {style} }}\n"));
    }
    css.push_str("</style>\n");

    // Substitute inline forms with class refs. The replace is literal
    // so we won't touch any `<span>` we didn't generate (e.g. user-
    // authored raw HTML inside cell text — but our exporter escapes
    // such things, so the concern is academic).
    let mut rewritten = html.to_string();
    for (style, cls) in &class_map {
        let from = format!(r#"<span style="{style}">"#);
        let to = format!(r#"<span class="{cls}">"#);
        rewritten = rewritten.replace(&from, &to);
    }
    format!("{css}{rewritten}")
}

fn emit_section_open(section: &Section, out: &mut String, opts: &HtmlOptions) {
    if opts.emit_pages {
        out.push_str(&page_open_tag(&section.properties));
    } else {
        out.push_str("<section>\n");
    }
}

fn emit_paragraph(doc: &IrDocument, para: &Paragraph, out: &mut String, opts: &HtmlOptions) {
    let body = render_para_text(doc, para, opts);
    let has_text = body_has_visible(&body);
    if has_text {
        match heading_level(doc, para) {
            Some(level) => {
                let tag = format!("h{}", level.clamp(1, 6));
                out.push('<');
                out.push_str(&tag);
                out.push('>');
                out.push_str(&body);
                out.push_str("</");
                out.push_str(&tag);
                out.push_str(">\n");
            }
            None => {
                out.push_str("<p>");
                out.push_str(&body);
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

/// Pick between styled and plain rendering. Styled goes through
/// `styled_text_html` (character-level walk with inline tags),
/// plain uses `clean_text` + `escape_html` — which is also the
/// correct fallback when the paragraph has no CharShapeRuns.
fn render_para_text(doc: &IrDocument, para: &Paragraph, opts: &HtmlOptions) -> String {
    if opts.emit_styles && !para.char_shape_runs.is_empty() {
        styled_text_html(
            &para.text,
            &para.char_shape_runs,
            &doc.doc_info.char_shapes,
            &doc.doc_info.font_faces,
        )
    } else {
        escape_html(&clean_text(&para.text))
    }
}

/// Whether a styled-or-plain HTML body contains any visible content
/// beyond whitespace and tags. Prevents emitting an empty `<p></p>`
/// when a paragraph's char-shape-driven span happens to collapse to
/// nothing (e.g. the paragraph was just U+FFFC placeholders).
fn body_has_visible(s: &str) -> bool {
    let mut in_tag = false;
    for c in s.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag && !c.is_whitespace() {
            return true;
        }
    }
    false
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
    let mut wrote_text = false;
    for p in cell.paragraphs.iter() {
        let body = render_para_text(doc, p, opts);
        if body_has_visible(&body) {
            if wrote_text {
                out.push_str("<br>");
            }
            out.push_str(&body);
            wrote_text = true;
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

/// Character-level walk that mirrors [`clean_text`] (same UTF-16 offset
/// math, same NBSP/FFFC/PUA translations, same whitespace rules inside
/// a run) but emits inline HTML tags for each `CharShapeRun`:
///
///   * `<span style="…">` — when the referenced shape's colour, font
///     size, or Hangul-slot font family differs from HWP defaults.
///   * `<s>` — strike
///   * `<strong>` — bold
///   * `<em>` — italic
///
/// Nesting order is strike > bold > italic (outer → inner), matching
/// the Markdown exporter's wrapper order so semantics line up across
/// the two outputs. The `<span>` (if any) wraps everything so CSS
/// styles apply across the whole run including nested emphasis.
fn styled_text_html(
    text: &str,
    runs: &[CharShapeRun],
    shapes: &[CharShape],
    fonts: &FontFaces,
) -> String {
    if runs.is_empty() {
        return escape_html(&clean_text(text));
    }
    let mut out = String::new();
    let mut u16_pos: u32 = 0;
    let mut open_tags: Vec<&'static str> = Vec::new();
    let mut span_open = false;
    let mut active_shape_id: Option<u32> = None;

    for c in text.chars() {
        let c_len = c.len_utf16() as u32;
        let new_shape_id = runs
            .iter()
            .rev()
            .find(|r| r.start <= u16_pos)
            .map(|r| r.char_shape_id);

        if new_shape_id != active_shape_id {
            while let Some(t) = open_tags.pop() {
                out.push_str(t);
            }
            if span_open {
                out.push_str("</span>");
                span_open = false;
            }
            if let Some(sid) = new_shape_id {
                if let Some(shape) = shapes.get(sid as usize) {
                    let style = compute_span_style(shape, fonts);
                    if !style.is_empty() {
                        out.push_str("<span style=\"");
                        out.push_str(&style);
                        out.push_str("\">");
                        span_open = true;
                    }
                    if shape.strike() {
                        out.push_str("<s>");
                        open_tags.push("</s>");
                    }
                    if shape.bold() {
                        out.push_str("<strong>");
                        open_tags.push("</strong>");
                    }
                    if shape.italic() {
                        out.push_str("<em>");
                        open_tags.push("</em>");
                    }
                }
            }
            active_shape_id = new_shape_id;
        }

        match c {
            '\u{FFFC}' | '\u{00AD}' => {}
            '\u{00A0}' | '\u{2003}' => out.push(' '),
            _ => {
                let t = translate_pua_bullet(c).unwrap_or(c);
                match t {
                    '&' => out.push_str("&amp;"),
                    '<' => out.push_str("&lt;"),
                    '>' => out.push_str("&gt;"),
                    '"' => out.push_str("&quot;"),
                    '\'' => out.push_str("&#39;"),
                    _ => out.push(t),
                }
            }
        }
        u16_pos += c_len;
    }
    while let Some(t) = open_tags.pop() {
        out.push_str(t);
    }
    if span_open {
        out.push_str("</span>");
    }
    out
}

/// Build the CSS declarations inside a span's `style="…"` for a shape.
/// Empty when the shape matches HWP defaults in all of colour (black),
/// font size (10 pt), and resolvable Hangul-slot font family — so the
/// caller can skip emitting an empty `<span>` entirely.
fn compute_span_style(shape: &CharShape, fonts: &FontFaces) -> String {
    let mut parts: Vec<String> = Vec::new();

    if shape.color != 0 {
        let r = (shape.color & 0xFF) as u8;
        let g = ((shape.color >> 8) & 0xFF) as u8;
        let b = ((shape.color >> 16) & 0xFF) as u8;
        parts.push(format!("color:#{:02x}{:02x}{:02x}", r, g, b));
    }

    // CharShape.base_size is in 1/100 pt. HWP's default body size is
    // 10 pt (1000), so skip when it matches — avoids a span on every
    // default-styled run.
    if shape.base_size != 1000 {
        let pt = (shape.base_size as f32) / 100.0;
        if (pt.fract()).abs() < 0.05 {
            parts.push(format!("font-size:{}pt", pt as i32));
        } else {
            parts.push(format!("font-size:{:.1}pt", pt));
        }
    }

    if let Some(name) = resolve_font_name(shape, fonts) {
        if !name.is_empty() {
            parts.push(format!("font-family:'{}'", escape_css_font(name)));
        }
    }

    parts.join(";")
}

/// Look up the shape's primary font name through the Hangul slot
/// (`font_ids[0]`). Other slots (latin/hanja/...) are intentionally
/// ignored: CSS `font-family` is a single fallback list and HWP's
/// script-switching model doesn't map cleanly onto the web. Hangul is
/// the dominant slot for Korean documents, so use it as the surface
/// signal and let the browser fall back.
fn resolve_font_name<'a>(shape: &CharShape, fonts: &'a FontFaces) -> Option<&'a str> {
    let id = shape.font_ids[0] as usize;
    fonts.hangul.get(id).map(|f| f.name.as_str())
}

/// Strip characters that would break a single-quoted `font-family`
/// attribute value. Real CSS escaping is finicky (backslash sequences);
/// for preview use the conservative approach of dropping the two
/// problem characters — fonts legitimately containing `'` or `\` are
/// vanishingly rare in HWP documents.
fn escape_css_font(s: &str) -> String {
    s.chars().filter(|c| *c != '\'' && *c != '\\').collect()
}

/// Emit `<section class="hwp-page" style="width:…mm;height:…mm;padding:
/// Tmm Rmm Bmm Lmm">`. When the section's properties carry zero
/// width/height (defaulted section with no PageDef decoded), fall back
/// to a plain `hwp-page` class with no inline dimensions so downstream
/// CSS can still target it.
fn page_open_tag(props: &SectionProperties) -> String {
    if props.page_width_hwpu == 0 || props.page_height_hwpu == 0 {
        return "<section class=\"hwp-page\">\n".into();
    }
    let w = hwpunit_to_mm(props.page_width_hwpu);
    let h = hwpunit_to_mm(props.page_height_hwpu);
    let [t, r, b, l] = props.margins_hwpu;
    format!(
        "<section class=\"hwp-page\" style=\"width:{}mm;height:{}mm;padding:{}mm {}mm {}mm {}mm\">\n",
        w,
        h,
        hwpunit_to_mm(t),
        hwpunit_to_mm(r),
        hwpunit_to_mm(b),
        hwpunit_to_mm(l)
    )
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
            &HtmlOptions { assets_path: Some("x.assets".into()), ..Default::default() },
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

    fn styled_doc(text: &str, runs: Vec<CharShapeRun>, shapes: Vec<CharShape>) -> IrDocument {
        let mut doc = IrDocument::default();
        doc.doc_info.styles = vec![style("본문")];
        doc.doc_info.char_shapes = shapes;
        doc.sections.push(Section {
            paragraphs: vec![Paragraph {
                header: ParagraphHeader { style_id: 0, ..ParagraphHeader::default() },
                text: text.into(),
                char_shape_runs: runs,
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        doc
    }

    fn bold_shape() -> CharShape {
        let mut s = CharShape::default();
        s.base_size = 1000;
        s.attr = 0x0000_0002; // bold bit
        s
    }

    fn italic_shape() -> CharShape {
        let mut s = CharShape::default();
        s.base_size = 1000;
        s.attr = 0x0000_0001; // italic bit
        s
    }

    fn strike_shape() -> CharShape {
        let mut s = CharShape::default();
        s.base_size = 1000;
        s.attr = 1 << 21;
        s
    }

    fn default_shape() -> CharShape {
        let mut s = CharShape::default();
        s.base_size = 1000;
        s
    }

    #[test]
    fn emit_styles_off_ignores_char_shape_runs() {
        let doc = styled_doc(
            "굵게 보통",
            vec![
                CharShapeRun { start: 0, char_shape_id: 0 },
                CharShapeRun { start: 2, char_shape_id: 1 },
            ],
            vec![bold_shape(), default_shape()],
        );
        let html = to_html(&doc);
        assert!(!html.contains("<strong>"), "emit_styles=false but got: {html}");
        assert!(html.contains("굵게 보통"), "got: {html}");
    }

    #[test]
    fn emit_styles_wraps_bold_italic_strike() {
        let doc = styled_doc(
            "ABCD",
            vec![
                CharShapeRun { start: 0, char_shape_id: 0 }, // bold
                CharShapeRun { start: 1, char_shape_id: 1 }, // italic
                CharShapeRun { start: 2, char_shape_id: 2 }, // strike
                CharShapeRun { start: 3, char_shape_id: 3 }, // default
            ],
            vec![bold_shape(), italic_shape(), strike_shape(), default_shape()],
        );
        let html = to_html_with(&doc, &HtmlOptions { emit_styles: true, ..Default::default() });
        assert!(html.contains("<strong>A</strong>"), "got: {html}");
        assert!(html.contains("<em>B</em>"), "got: {html}");
        assert!(html.contains("<s>C</s>"), "got: {html}");
        // Default run after strike emits plain text.
        assert!(html.contains("</s>D"), "default run should be unwrapped: {html}");
    }

    #[test]
    fn emit_styles_colored_run_gets_span_with_hex_color() {
        let mut colored = default_shape();
        colored.color = 0x00_00_00_FF; // R = 0xFF, G = 0x00, B = 0x00 → red
        let doc = styled_doc(
            "X",
            vec![CharShapeRun { start: 0, char_shape_id: 0 }],
            vec![colored],
        );
        let html = to_html_with(&doc, &HtmlOptions { emit_styles: true, ..Default::default() });
        assert!(
            html.contains("<span style=\"color:#ff0000\">X</span>"),
            "got: {html}"
        );
    }

    #[test]
    fn emit_styles_nondefault_font_size_emits_span() {
        let mut big = default_shape();
        big.base_size = 1500; // 15 pt
        let doc = styled_doc(
            "X",
            vec![CharShapeRun { start: 0, char_shape_id: 0 }],
            vec![big],
        );
        let html = to_html_with(&doc, &HtmlOptions { emit_styles: true, ..Default::default() });
        assert!(html.contains("font-size:15pt"), "got: {html}");
    }

    #[test]
    fn emit_styles_default_shape_emits_no_span() {
        let doc = styled_doc(
            "ABC",
            vec![CharShapeRun { start: 0, char_shape_id: 0 }],
            vec![default_shape()],
        );
        let html = to_html_with(&doc, &HtmlOptions { emit_styles: true, ..Default::default() });
        // No colour, default size, no font-face data → no span, no tag noise.
        assert!(!html.contains("<span"), "default shape shouldn't emit span: {html}");
        assert!(html.contains("<p>ABC</p>"), "got: {html}");
    }

    #[test]
    fn emit_styles_font_family_from_hangul_slot() {
        use hwp_transpiler_core::ir::FontFace;
        let mut doc = styled_doc(
            "가",
            vec![CharShapeRun { start: 0, char_shape_id: 0 }],
            vec![default_shape()],
        );
        doc.doc_info.font_faces.hangul.push(FontFace {
            properties: 0,
            name: "함초롬바탕".into(),
            substitute: None,
            type_info: None,
            default_name: None,
        });
        let html = to_html_with(&doc, &HtmlOptions { emit_styles: true, ..Default::default() });
        assert!(
            html.contains("font-family:'함초롬바탕'"),
            "got: {html}"
        );
    }

    #[test]
    fn emit_pages_off_keeps_plain_section() {
        let doc = make_doc(vec![style("본문")], vec![para(0, "hi")]);
        let html = to_html(&doc);
        assert!(html.contains("<section>\n"), "got: {html}");
        assert!(!html.contains("hwp-page"), "got: {html}");
    }

    #[test]
    fn emit_pages_on_with_dims_wraps_hwp_page_section() {
        let mut doc = make_doc(vec![style("본문")], vec![para(0, "hi")]);
        // A4 portrait ≈ 59528 × 84168 HWPUNIT, margins 20mm / 15mm / 20mm / 15mm.
        doc.sections[0].properties = SectionProperties {
            page_width_hwpu: 59528,
            page_height_hwpu: 84168,
            margins_hwpu: [5669, 4251, 5669, 4251],
            columns: 1,
        };
        let html = to_html_with(&doc, &HtmlOptions { emit_pages: true, ..Default::default() });
        assert!(
            html.contains("<section class=\"hwp-page\""),
            "got: {html}"
        );
        assert!(html.contains("width:210mm"), "got: {html}");
        assert!(html.contains("height:297mm"), "got: {html}");
        assert!(html.contains("padding:20mm 15mm 20mm 15mm"), "got: {html}");
    }

    #[test]
    fn emit_pages_on_with_zero_dims_falls_back_to_plain_class() {
        let doc = make_doc(vec![style("본문")], vec![para(0, "hi")]);
        let html = to_html_with(&doc, &HtmlOptions { emit_pages: true, ..Default::default() });
        // Class still present so CSS can target; no inline style when
        // no real dimensions were decoded.
        assert!(html.contains("<section class=\"hwp-page\">"), "got: {html}");
        assert!(!html.contains("width:0mm"), "got: {html}");
    }

    #[test]
    fn repeated_inline_styles_get_hoisted_to_class() {
        // Three paragraphs sharing the same colored shape → the
        // inline style repeats three times before hoisting and
        // collapses to a single class rule afterwards.
        let mut colored = default_shape();
        colored.color = 0x00_00_CC_FF; // blue
        let mut doc = IrDocument::default();
        doc.doc_info.styles = vec![style("본문")];
        doc.doc_info.char_shapes = vec![colored];
        let make_run_para = |t: &str| Paragraph {
            header: ParagraphHeader { style_id: 0, ..ParagraphHeader::default() },
            text: t.into(),
            char_shape_runs: vec![CharShapeRun { start: 0, char_shape_id: 0 }],
            ..Paragraph::default()
        };
        doc.sections.push(Section {
            paragraphs: vec![
                make_run_para("aaa"),
                make_run_para("bbb"),
                make_run_para("ccc"),
            ],
            ..Section::default()
        });
        let html = to_html_with(
            &doc,
            &HtmlOptions { emit_styles: true, ..Default::default() },
        );
        // Stylesheet block appears once.
        assert!(html.starts_with("<style>\n"), "got: {html}");
        // CSS rule scoped under .hwp-preview.
        assert!(html.contains(".hwp-preview .s0"), "got: {html}");
        // Body spans now reference the class, not the literal color.
        assert!(html.contains(r#"<span class="s0">"#), "got: {html}");
        // No stragglers left with the original inline form.
        assert!(
            !html.contains(r#"<span style="color:#ff0000""#),
            "no remaining inline: {html}"
        );
    }

    #[test]
    fn single_occurrence_style_stays_inline() {
        // Only one paragraph with a unique style → no benefit to
        // hoisting, keep it inline.
        let mut colored = default_shape();
        colored.color = 0x00_00_00_FF; // red
        let doc = styled_doc(
            "X",
            vec![CharShapeRun { start: 0, char_shape_id: 0 }],
            vec![colored],
        );
        let html = to_html_with(
            &doc,
            &HtmlOptions { emit_styles: true, ..Default::default() },
        );
        assert!(!html.starts_with("<style>"), "shouldn't hoist singletons");
        assert!(
            html.contains(r#"<span style="color:#ff0000">"#),
            "inline kept: {html}"
        );
    }
}
