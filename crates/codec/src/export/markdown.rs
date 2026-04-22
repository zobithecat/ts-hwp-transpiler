//! HWP → Markdown export.
//!
//! Walks sections → paragraphs and emits:
//!   - Body text (with heading detection via Style name)
//!   - Inline tables as:
//!       * **Simple** (regular grid, no merge, no nested tables, no multi-
//!         paragraph cells) → standard Markdown table
//!       * **Complex** (ragged row counts, merged cells, nested tables, or
//!         multi-paragraph cells) → nested bullet list per cell with
//!         `[row,col]` and optional `span R×C` tags
//!
//! The complex-case heuristic follows the task spec: "복잡한 병합 표는
//! 표준 MD 표 대신 Nested Bullet List로 변환".

use hwp_transpiler_core::ir::{
    ControlKind, IrDocument, Paragraph, PictureControl, TableCell, TableControl,
};

/// Knobs the caller passes to `to_markdown_with`. The bare `to_markdown`
/// uses defaults (no asset link emitted; pictures collapse to bare
/// `{{그림 N.}}` placeholders).
#[derive(Debug, Clone, Default)]
pub struct MdOptions {
    /// Relative URL prefix for image references — typically the sidecar
    /// asset directory. When set, each top-level picture emits
    /// `![](<prefix>/BIN<id>.<ext>){width=Xmm; height=Ymm}` followed by
    /// the `{{그림 N.}}` placeholder. When `None`, only the placeholder
    /// is written.
    pub assets_path: Option<String>,
    /// When `Some`, `to_markdown_with` dispatches to the LLM-friendly
    /// structured emitter (see `markdown_llm`) instead of the human
    /// Markdown path. Opt-in so existing callers keep the same output.
    pub llm: Option<LlmOptions>,
}

/// Capability flags for the LLM-friendly Markdown layer. Kept minimal on
/// purpose — the first skeleton always emits stable ids; role /
/// editable / domain-hint annotations land as follow-up fields and
/// default to off so existing snapshots stay stable.
#[derive(Debug, Clone, Default)]
pub struct LlmOptions {
    /// Emit `role=label|value|unknown` on each CELL marker. Off by
    /// default — heuristic not wired yet; enabling it today is a no-op
    /// placeholder so the flag can be plumbed in before the classifier
    /// lands.
    pub emit_roles: bool,
    /// Emit `editable=true|false|unknown` on each CELL marker. Same
    /// placeholder status as `emit_roles`.
    pub emit_editable: bool,
}

pub fn to_markdown(doc: &IrDocument) -> String {
    to_markdown_with(doc, &MdOptions::default())
}

pub fn to_markdown_with(doc: &IrDocument, opts: &MdOptions) -> String {
    if opts.llm.is_some() {
        return super::markdown_llm::to_llm_markdown(doc, opts);
    }
    let mut out = String::new();
    let mut picture_counter: u32 = 0;
    for section in &doc.sections {
        for para in &section.paragraphs {
            emit_paragraph(doc, para, &mut out, 0, opts, &mut picture_counter);
        }
    }
    while out.ends_with(|c: char| c.is_whitespace()) {
        out.pop();
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

fn emit_paragraph(
    doc: &IrDocument,
    para: &Paragraph,
    out: &mut String,
    depth: usize,
    opts: &MdOptions,
    picture_counter: &mut u32,
) {
    let text = clean_text(&para.text);
    if !text.is_empty() {
        match heading_level(doc, para) {
            Some(level) => {
                out.push_str(&"#".repeat(level.clamp(1, 6) as usize));
                out.push(' ');
                out.push_str(&text);
            }
            None => out.push_str(&text),
        }
        out.push_str("\n\n");
    }
    for c in &para.controls {
        match &c.kind {
            ControlKind::Table(t) => emit_table(doc, t, out, depth, opts, picture_counter),
            ControlKind::Picture(p) => {
                *picture_counter += 1;
                emit_picture(doc, p, out, opts, *picture_counter);
            }
            _ => {}
        }
    }
}

/// Build the `![](prefix/BIN<id>.<ext>){width=Xmm; height=Ymm}` line for
/// a picture. Returns `None` when `assets_path` is unset — callers then
/// emit only the placeholder.
fn picture_image_line(
    doc: &IrDocument,
    pic: &PictureControl,
    opts: &MdOptions,
) -> Option<String> {
    let prefix = opts.assets_path.as_deref()?;
    let ext = resolve_bin_extension(doc, pic.bin_id);
    let filename = format!("BIN{:04}.{}", pic.bin_id, ext);
    let w = hwpunit_to_mm(pic.width_hwpu);
    let h = hwpunit_to_mm(pic.height_hwpu);
    Some(format!(
        "![]({prefix}/{filename}){{width={w}mm; height={h}mm}}"
    ))
}

/// Build the `{{그림 N. <caption>}}` placeholder string (no trailing
/// newline). Caption text is cleaned and the HWP auto-numbering
/// `"그림 . "` prefix — left stranded after `clean_text` drops U+FFFC —
/// is removed.
fn picture_caption_line(pic: &PictureControl, n: u32) -> String {
    let caption_suffix = pic
        .caption_text
        .as_deref()
        .map(clean_text)
        .map(|s| strip_caption_label_prefix(&s).trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|s| format!(" {s}"))
        .unwrap_or_default();
    format!("{{{{그림 {n}.{caption_suffix}}}}}")
}

/// Emit one top-level picture as its own block: optional image ref
/// followed by the caption placeholder, each separated by the usual
/// paragraph blank line.
fn emit_picture(
    doc: &IrDocument,
    pic: &PictureControl,
    out: &mut String,
    opts: &MdOptions,
    n: u32,
) {
    if let Some(img) = picture_image_line(doc, pic, opts) {
        out.push_str(&img);
        out.push_str("\n\n");
    }
    out.push_str(&picture_caption_line(pic, n));
    out.push_str("\n\n");
}

/// Emit a picture as sub-bullets of a bullet-list cell. Keeps the list
/// structure intact — no blank-line separators that would break out of
/// the parent `- [r,c]:` item — and surfaces both the image ref and the
/// caption placeholder in order.
fn emit_picture_bullet(
    indent: &str,
    doc: &IrDocument,
    pic: &PictureControl,
    out: &mut String,
    opts: &MdOptions,
    n: u32,
) {
    if let Some(img) = picture_image_line(doc, pic, opts) {
        out.push_str(indent);
        out.push_str("- ");
        out.push_str(&img);
        out.push('\n');
    }
    out.push_str(indent);
    out.push_str("- ");
    out.push_str(&picture_caption_line(pic, n));
    out.push('\n');
}

/// HWP captions authored via the built-in "그림" / "표" auto-numbering
/// field produce text like `"그림 ￼. <title>"` where `￼` (U+FFFC) is the
/// field-code placeholder for the running figure number. `clean_text`
/// drops the FFFC, leaving a stranded `"그림 . "` prefix that duplicates
/// (and clashes with) our own `{{그림 N.}}` counter. Strip it so the
/// emitted placeholder reads as `{{그림 N. <title>}}`. Unknown-language
/// forms ("Figure", "Table", English "표" alias) covered symmetrically.
pub(super) fn strip_caption_label_prefix(s: &str) -> &str {
    for prefix in ["그림 . ", "표 . ", "Figure . ", "Table . "] {
        if let Some(rest) = s.strip_prefix(prefix) {
            return rest;
        }
    }
    s
}

/// Extension for `BIN<bin_id>` — looked up in the typed DocInfo BinData
/// records. Returns `"bin"` as a safe fallback if the matching record is
/// missing or has no extension (e.g. external link, or pre-Phase-1 IR).
fn resolve_bin_extension(doc: &IrDocument, bin_id: u16) -> &str {
    doc.doc_info
        .bin_data
        .iter()
        .find(|bd| bd.bin_data_id == Some(bin_id))
        .and_then(|bd| bd.extension.as_deref())
        .unwrap_or("bin")
}

/// HWPUNIT → millimetres (rounded). Definition: 1 HWPUNIT = 1/7200 inch
/// (Page Unit Inch), so `mm = hwpu × 25.4 / 7200`. Image dimensions are
/// always positive in the IR, so a u32-out is safe.
fn hwpunit_to_mm(hwpu: u32) -> u32 {
    ((hwpu as f64) * 25.4 / 7200.0).round() as u32
}

fn emit_table(
    doc: &IrDocument,
    t: &TableControl,
    out: &mut String,
    depth: usize,
    opts: &MdOptions,
    picture_counter: &mut u32,
) {
    if let Some(inner) = try_unwrap_wrapper_table(t) {
        // Pure decorative wrapper (1×1 with one nested table, no own
        // text). Skip the outer frame — emit the inner directly at the
        // same depth so it doesn't gain a useless extra indent.
        emit_table(doc, inner, out, depth, opts, picture_counter);
        return;
    }
    if let Some((level, text)) = try_table_as_heading(t) {
        out.push_str(&"#".repeat(level as usize));
        out.push(' ');
        out.push_str(&text);
        out.push_str("\n\n");
        return;
    }
    if depth == 0 {
        if let Some(passage) = try_table_as_passage(t) {
            out.push_str(&passage);
            out.push_str("\n\n");
            return;
        }
    }
    if let Some(grid) = try_build_md_grid(t) {
        emit_md_grid(&grid, out);
    } else {
        emit_table_as_list(doc, t, out, depth, opts, picture_counter);
    }
}

/// Detect a single-row table whose only meaningful content is one short
/// line of text (the rest of the cells, if any, are empty). HWP frequently
/// uses such tables purely for visual framing — a doc title in a bordered
/// box, an interstitial section banner — and they don't read as tables in
/// Markdown. Emit them as plain prose instead.
///
/// Limited to top-level (`depth == 0`) so nested headers inside complex
/// tables still render inside their parent's bullet structure.
fn try_table_as_passage(t: &TableControl) -> Option<String> {
    if t.rows != 1 {
        return None;
    }
    let non_empty: Vec<&TableCell> = t
        .cells
        .iter()
        .filter(|c| c.paragraphs.iter().any(|p| !clean_text(&p.text).is_empty()))
        .collect();
    if non_empty.len() != 1 {
        return None;
    }
    let cell = non_empty[0];
    // Reject anything with controls (nested tables, pictures, equations).
    if cell.paragraphs.iter().any(|p| !p.controls.is_empty()) {
        return None;
    }
    // Multi-paragraph short cells (HWP often splits a title across two
    // paragraphs for line wrapping) are fine — join with spaces.
    let parts: Vec<String> = cell
        .paragraphs
        .iter()
        .map(|p| clean_text(&p.text))
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }
    let text = parts.join(" ");
    if text.contains('\n') || text.chars().count() > 100 {
        return None;
    }
    Some(text)
}

/// Detect a "decorative wrapper" 1×1 table: one cell, no body text, and
/// exactly one inline table as the only control. HWP often nests a real
/// table inside such a wrapper to inherit a border/background style; for
/// us the wrapper just adds a useless `- [0,0]:` line and indent depth.
fn try_unwrap_wrapper_table(t: &TableControl) -> Option<&TableControl> {
    if t.cells.len() != 1 {
        return None;
    }
    let cell = &t.cells[0];
    if cell.paragraphs.iter().any(|p| !clean_text(&p.text).is_empty()) {
        return None;
    }
    let mut nested: Option<&TableControl> = None;
    for p in &cell.paragraphs {
        for c in &p.controls {
            match &c.kind {
                ControlKind::Table(inner) => {
                    if nested.is_some() {
                        return None;
                    }
                    nested = Some(inner);
                }
                // Tolerate opaque inline markers (CTRL_HEADER codes we
                // haven't typed yet — anchors, page breaks, etc.); they
                // don't carry user-visible content. Reject anything we
                // *do* know about (Picture / Equation) since dropping
                // those would lose data.
                ControlKind::Unknown { .. } => {}
                _ => return None,
            }
        }
    }
    nested
}

/// Detect HWP's "decorative box heading" pattern: a table with exactly one
/// non-empty cell whose text is a short line beginning with a numeric
/// prefix (`1. ...`) or parenthetical (`(1) ...`). This is how 한컴 documents
/// typically render top-level section headers — borders + padded box around
/// the title — and the only signal we have without DocInfo style names.
///
/// Accepts multi-paragraph cells (joined with spaces) because HWP authors
/// occasionally break a long title across paragraphs for visual line
/// wrapping — observed in the TRL fixture's chapter-7 heading where
/// `7. 연구개발성과의 활용방안 및 기대효과` and
/// `(기술성·시장성 및 사업성 검토 방안 등)` sit in the same cell as
/// paragraph 0 and paragraph 1. A strict one-paragraph gate dropped such
/// cases to the passage path and erased the `##` marker.
fn try_table_as_heading(t: &TableControl) -> Option<(u8, String)> {
    let non_empty: Vec<&TableCell> = t
        .cells
        .iter()
        .filter(|c| c.paragraphs.iter().any(|p| !clean_text(&p.text).is_empty()))
        .collect();
    if non_empty.len() != 1 {
        return None;
    }
    let cell = non_empty[0];
    // Any control anywhere in the cell (nested table, picture) would be
    // lost if we collapse to a heading — reject.
    if cell.paragraphs.iter().any(|p| !p.controls.is_empty()) {
        return None;
    }
    let parts: Vec<String> = cell
        .paragraphs
        .iter()
        .map(|p| clean_text(&p.text))
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }
    let text = parts.join(" ");
    if text.contains('\n') || text.chars().count() > 80 {
        return None;
    }
    let level = heading_level_from_prefix(&text)?;
    Some((level, text))
}

/// Map "1. X" → ##, "(1) X" → ###. Returns `None` when no prefix matches —
/// that's intentional: we'd rather miss a heading than promote arbitrary
/// short box text to a section break.
fn heading_level_from_prefix(s: &str) -> Option<u8> {
    let s = s.trim_start();

    if let Some(idx) = s.find('.') {
        let head = &s[..idx];
        let after = &s[idx + '.'.len_utf8()..];
        if !head.is_empty()
            && head.chars().all(|c| c.is_ascii_digit())
            && after.starts_with(' ')
        {
            return Some(2);
        }
    }

    if let Some(rest) = s.strip_prefix('(') {
        if let Some(end) = rest.find(") ") {
            let inside = &rest[..end];
            if !inside.is_empty()
                && inside
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || ('가'..='힣').contains(&c))
            {
                return Some(3);
            }
        }
    }

    None
}

/// Try to fold a table into a Markdown grid. Succeeds when:
///
///   - all `row_span` are 1 (GFM has no rowspan, so any vertical merge
///     forces the bullet path),
///   - cells (after expanding `col_span`) tile the grid without overlaps
///     and without holes,
///   - no cell contains a nested table (those need their own block).
///
/// Returns the rectangular grid; merged-cell extents are filled with empty
/// strings so the row still has the right column count.
fn try_build_md_grid(t: &TableControl) -> Option<Vec<Vec<String>>> {
    let rows = t.rows as usize;
    let cols = t.cols as usize;
    if rows == 0 || cols == 0 {
        return None;
    }
    let mut grid: Vec<Vec<Option<String>>> = vec![vec![None; cols]; rows];
    for cell in &t.cells {
        if cell.row_span != 1 {
            return None;
        }
        let r = cell.row as usize;
        let c = cell.col as usize;
        let cs = (cell.col_span as usize).max(1);
        if r >= rows || c + cs > cols {
            return None;
        }
        // GFM grid can't host nested blocks; pictures and nested tables
        // both force the bullet path so their structure survives.
        for p in &cell.paragraphs {
            for ctrl in &p.controls {
                if matches!(
                    &ctrl.kind,
                    ControlKind::Table(_) | ControlKind::Picture(_)
                ) {
                    return None;
                }
            }
        }
        let text = cell_text_inline(cell);
        if grid[r][c].replace(text).is_some() {
            return None;
        }
        for k in 1..cs {
            if grid[r][c + k].replace(String::new()).is_some() {
                return None;
            }
        }
    }
    let mut full: Vec<Vec<String>> = Vec::with_capacity(rows);
    for row in grid {
        let mut new_row = Vec::with_capacity(cols);
        for col in row {
            new_row.push(col?);
        }
        full.push(new_row);
    }
    Some(full)
}

fn emit_md_grid(grid: &[Vec<String>], out: &mut String) {
    if grid.is_empty() || grid[0].is_empty() {
        return;
    }
    let cols = grid[0].len();
    write_row(&grid[0], out);
    out.push('|');
    for _ in 0..cols {
        out.push_str(" --- |");
    }
    out.push('\n');
    for row in grid.iter().skip(1) {
        write_row(row, out);
    }
    out.push('\n');
}

fn write_row(row: &[String], out: &mut String) {
    out.push('|');
    for cell in row {
        out.push(' ');
        out.push_str(cell);
        out.push_str(" |");
    }
    out.push('\n');
}

/// Inline form of a cell's paragraphs, suitable for a single MD table cell:
/// paragraphs joined with spaces, pipes/newlines escaped.
fn cell_text_inline(cell: &TableCell) -> String {
    let mut text = String::new();
    for (i, p) in cell.paragraphs.iter().enumerate() {
        if i > 0 {
            text.push(' ');
        }
        text.push_str(&clean_text(&p.text));
    }
    text.replace('|', "\\|").replace('\n', " ")
}

fn emit_table_as_list(
    doc: &IrDocument,
    t: &TableControl,
    out: &mut String,
    depth: usize,
    opts: &MdOptions,
    picture_counter: &mut u32,
) {
    let indent = "  ".repeat(depth);
    out.push_str(&indent);
    out.push_str(&format!(
        "<!-- table {}×{} ({}) -->\n",
        t.rows,
        t.cols,
        if t.cells.len() as u32
            == t.row_cell_counts.iter().map(|&n| n as u32).sum::<u32>()
        {
            "ragged/merged"
        } else {
            "complex"
        }
    ));

    // Pre-pass: collapse runs of unspanned empty cells in the same row into
    // a single `[r,c1..c2]: (empty)` line. Empty Gantt-style spreader cells
    // can otherwise dominate the bullet list (one row × 9 cols = 9 lines of
    // noise).
    let mut i = 0;
    while i < t.cells.len() {
        let cell = &t.cells[i];
        if is_simple_empty_cell(cell) {
            let mut end_col = cell.col;
            let mut j = i + 1;
            while j < t.cells.len() {
                let next = &t.cells[j];
                if next.row == cell.row
                    && next.col == end_col + 1
                    && is_simple_empty_cell(next)
                {
                    end_col = next.col;
                    j += 1;
                } else {
                    break;
                }
            }
            if j > i + 1 {
                out.push_str(&indent);
                out.push_str(&format!(
                    "- [{},{}..{}]: (empty)\n",
                    cell.row, cell.col, end_col
                ));
                i = j;
                continue;
            }
        }
        emit_cell_line(doc, cell, out, &indent, depth, opts, picture_counter);
        i += 1;
    }

    out.push('\n');
}

fn emit_cell_line(
    doc: &IrDocument,
    cell: &TableCell,
    out: &mut String,
    indent: &str,
    depth: usize,
    opts: &MdOptions,
    picture_counter: &mut u32,
) {
    out.push_str(indent);
    out.push_str("- ");
    out.push_str(&format!("[{},{}]", cell.row, cell.col));
    if cell.col_span != 1 || cell.row_span != 1 {
        out.push_str(&format!(" span {}×{}", cell.row_span, cell.col_span));
    }
    out.push(':');

    let lines = cell_text_lines(cell);
    if lines.is_empty() {
        out.push('\n');
    } else {
        let inline = lines.join(" · ");
        // Inline ` · `-joined form when the cell is short enough that the
        // structure is obvious at a glance. For long passages — typical of
        // 한컴's "wrap a whole section in a 1×1 box" pattern — emit each
        // paragraph as its own sub-bullet so headings like `○`, `-`, and
        // numbered lists inside the cell remain visible.
        if lines.len() == 1 || inline.chars().count() <= INLINE_CELL_LIMIT {
            out.push(' ');
            out.push_str(&inline);
            out.push('\n');
        } else {
            out.push('\n');
            for line in &lines {
                out.push_str(indent);
                out.push_str("  - ");
                out.push_str(line);
                out.push('\n');
            }
        }
    }

    // Sub-indent for child blocks belonging to this cell line: two
    // spaces deeper than the bullet itself so Markdown associates them
    // with the parent list item.
    let child_indent = format!("{indent}  ");
    for p in &cell.paragraphs {
        for c in &p.controls {
            match &c.kind {
                ControlKind::Table(nested) => {
                    emit_table(doc, nested, out, depth + 1, opts, picture_counter);
                }
                ControlKind::Picture(pic) => {
                    *picture_counter += 1;
                    emit_picture_bullet(
                        &child_indent,
                        doc,
                        pic,
                        out,
                        opts,
                        *picture_counter,
                    );
                }
                _ => {}
            }
        }
    }
}

const INLINE_CELL_LIMIT: usize = 200;

fn is_simple_empty_cell(cell: &TableCell) -> bool {
    cell.col_span == 1
        && cell.row_span == 1
        && cell.paragraphs.iter().all(|p| {
            p.controls.is_empty() && clean_text(&p.text).is_empty()
        })
}

/// Per-paragraph text for a cell, with intra-paragraph newlines flattened
/// to spaces and empty paragraphs dropped. Used by the bullet path to
/// decide between inline ` · ` joining and nested sub-bullets.
fn cell_text_lines(cell: &TableCell) -> Vec<String> {
    cell.paragraphs
        .iter()
        .map(|p| clean_text(&p.text).replace('\n', " "))
        .filter(|s| !s.is_empty())
        .collect()
}

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

pub(super) fn clean_text(s: &str) -> String {
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

/// Map Hancom Office's proprietary PUA-encoded circled-digit bullets
/// (used by its Korean enumeration list styles) to standard Unicode
/// equivalents. Hancom uses two ranges in practice — older fonts encode
/// in the BMP PUA, newer ones in Supplementary PUA-A — and the same
/// `①` glyph appears at both `U+F2B1` and `U+F02B1`. Returns `None` for
/// anything outside the recognised range; callers fall back to the
/// original char.
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

#[cfg(test)]
mod tests {
    use super::*;
    use hwp_transpiler_core::ir::{
        Control, ControlKind, IrDocument, Paragraph, ParagraphHeader, Section, Style, TableCell,
        TableControl,
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

    fn para_with_table(t: TableControl) -> Paragraph {
        Paragraph {
            controls: vec![Control { kind: ControlKind::Table(t) }],
            ..Paragraph::default()
        }
    }

    #[test]
    fn heading_from_korean_style() {
        let doc = make_doc(
            vec![style("본문"), style("개요 1"), style("개요 3")],
            vec![para(0, "intro"), para(1, "Chapter One"), para(2, "Subsection")],
        );
        assert_eq!(
            to_markdown(&doc),
            "intro\n\n# Chapter One\n\n### Subsection\n"
        );
    }

    #[test]
    fn hancom_pua_circled_digits_map_to_unicode() {
        // 한컴 fonts encode ①..⑳ in two PUA ranges depending on the
        // font generation: BMP (U+F2B1+) and Supplementary PUA-A
        // (U+F02B1+). Both should normalise to U+2460+.
        let doc = make_doc(
            vec![style("본문")],
            vec![para(
                0,
                "\u{F2B1} 과제 개요\n\u{F02B2} 부록\n\u{F2BA} 마지막\n\u{F02C4} 끝",
            )],
        );
        let md = to_markdown(&doc);
        assert!(md.contains("① 과제 개요"), "got: {md}");
        assert!(md.contains("② 부록"), "got: {md}");
        assert!(md.contains("⑩ 마지막"), "got: {md}");
        assert!(md.contains("⑳ 끝"), "got: {md}");
        // No raw PUA chars left from either range.
        assert!(!md.contains('\u{F2B1}'));
        assert!(!md.contains('\u{F02B1}'));
    }

    #[test]
    fn strips_extended_control_and_nbsp() {
        let doc = make_doc(
            vec![style("본문")],
            vec![para(0, "hello\u{FFFC}\u{00A0}world")],
        );
        assert_eq!(to_markdown(&doc), "hello world\n");
    }

    #[test]
    fn simple_2x2_table_emits_md_table() {
        let t = TableControl {
            rows: 2,
            cols: 2,
            row_cell_counts: vec![2, 2],
            cells: vec![
                cell(0, 0, "a"),
                cell(1, 0, "b"),
                cell(0, 1, "c"),
                cell(1, 1, "d"),
            ],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(t)]);
        let md = to_markdown(&doc);
        assert!(md.contains("| a | b |"));
        assert!(md.contains("| --- | --- |"));
        assert!(md.contains("| c | d |"));
    }

    #[test]
    fn box_table_with_numeric_prefix_becomes_heading() {
        // 1×2 simple table where only the first cell has text — the very
        // common HWP pattern for top-level section titles.
        let t = TableControl {
            rows: 1,
            cols: 2,
            row_cell_counts: vec![2],
            cells: vec![
                cell(0, 0, "1. 기술개발 목표"),
                cell(1, 0, ""),
            ],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(t)]);
        assert_eq!(to_markdown(&doc), "## 1. 기술개발 목표\n");
    }

    #[test]
    fn strip_caption_label_prefix_removes_post_fffc_artifact() {
        // After clean_text drops U+FFFC, HWP's "그림 ￼. foo" becomes the
        // stranded "그림 . foo". Must collapse to "foo".
        assert_eq!(strip_caption_label_prefix("그림 . foo"), "foo");
        assert_eq!(strip_caption_label_prefix("표 . bar"), "bar");
        assert_eq!(strip_caption_label_prefix("Figure . baz"), "baz");
        assert_eq!(strip_caption_label_prefix("Table . qux"), "qux");
        // Without the exact prefix, leave untouched.
        assert_eq!(strip_caption_label_prefix("그림 설명"), "그림 설명");
        assert_eq!(strip_caption_label_prefix("foo bar"), "foo bar");
    }

    #[test]
    fn picture_emits_caption_in_placeholder() {
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
                        caption_text: Some(
                            "그림 \u{FFFC}. 시스템 전체 아키텍처".into(),
                        ),
                    }),
                }],
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        let md = to_markdown_with(
            &doc,
            &MdOptions {
                assets_path: Some("x.assets".into()),
                ..MdOptions::default()
            },
        );
        assert!(
            md.contains("{{그림 1. 시스템 전체 아키텍처}}"),
            "expected caption-suffixed placeholder; got: {md}"
        );
    }

    #[test]
    fn picture_inside_cell_emits_bullet_subitem() {
        use hwp_transpiler_core::ir::{BinData, PictureControl};

        let mut doc = IrDocument::default();
        doc.doc_info.bin_data.push(BinData {
            bin_data_id: Some(7),
            extension: Some("jpg".into()),
            ..BinData::default()
        });
        // 2×1 with row_span on col 0 to force the bullet path. Col 1
        // cell has a picture inside its paragraph.
        let cell_with_pic = TableCell {
            col: 1, row: 0, col_span: 1, row_span: 1,
            paragraphs: vec![Paragraph {
                text: "caption nearby".into(),
                controls: vec![Control {
                    kind: ControlKind::Picture(PictureControl {
                        bin_id: 7,
                        width_hwpu: 7200,
                        height_hwpu: 3600,
                        caption_text: Some("그림 \u{FFFC}. 연구팀 구성 도식".into()),
                    }),
                }],
                ..Paragraph::default()
            }],
            ..TableCell::default()
        };
        let t = TableControl {
            rows: 2, cols: 2, row_cell_counts: vec![2, 1],
            cells: vec![
                TableCell {
                    col: 0, row: 0, col_span: 1, row_span: 2,
                    paragraphs: vec![para(0, "anchor")],
                    ..TableCell::default()
                },
                cell_with_pic,
                cell(0, 1, "row2"),
            ],
            ..TableControl::default()
        };
        doc.doc_info.styles = vec![style("본문")];
        doc.sections.push(Section {
            paragraphs: vec![para_with_table(t)],
            ..Section::default()
        });
        let md = to_markdown_with(
            &doc,
            &MdOptions { assets_path: Some("x.assets".into()), ..MdOptions::default() },
        );
        assert!(md.contains("- ![](x.assets/BIN0007.jpg)"), "image bullet missing: {md}");
        assert!(
            md.contains("- {{그림 1. 연구팀 구성 도식}}"),
            "caption bullet missing / wrong number: {md}"
        );
    }

    #[test]
    fn picture_in_grid_candidate_forces_bullet_fallback() {
        // Without the picture, this 2×2 would render as a GFM grid.
        // With one, try_build_md_grid must reject so the picture has
        // somewhere to land.
        use hwp_transpiler_core::ir::PictureControl;
        let pic_cell = TableCell {
            col: 0, row: 0, col_span: 1, row_span: 1,
            paragraphs: vec![Paragraph {
                controls: vec![Control {
                    kind: ControlKind::Picture(PictureControl {
                        bin_id: 1, width_hwpu: 0, height_hwpu: 0,
                        caption_text: None,
                    }),
                }],
                ..Paragraph::default()
            }],
            ..TableCell::default()
        };
        let t = TableControl {
            rows: 2, cols: 2, row_cell_counts: vec![2, 2],
            cells: vec![pic_cell, cell(1, 0, "b"), cell(0, 1, "c"), cell(1, 1, "d")],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(t)]);
        let md = to_markdown(&doc);
        assert!(
            md.contains("<!-- table 2×2"),
            "picture should force bullet fallback, not grid: {md}"
        );
        assert!(md.contains("{{그림 1.}}"), "got: {md}");
    }

    #[test]
    fn picture_counter_spans_top_level_and_cell_pictures() {
        // Verifies the shared mutable counter: one top-level picture,
        // then a picture inside a table — must be N=1 and N=2 in order.
        use hwp_transpiler_core::ir::PictureControl;

        let top_pic = Paragraph {
            controls: vec![Control {
                kind: ControlKind::Picture(PictureControl {
                    bin_id: 1, width_hwpu: 0, height_hwpu: 0, caption_text: None,
                }),
            }],
            ..Paragraph::default()
        };
        let in_cell = TableCell {
            col: 1, row: 0, col_span: 1, row_span: 1,
            paragraphs: vec![Paragraph {
                controls: vec![Control {
                    kind: ControlKind::Picture(PictureControl {
                        bin_id: 2, width_hwpu: 0, height_hwpu: 0, caption_text: None,
                    }),
                }],
                ..Paragraph::default()
            }],
            ..TableCell::default()
        };
        let t = TableControl {
            rows: 2, cols: 2, row_cell_counts: vec![2, 1],
            cells: vec![
                TableCell {
                    col: 0, row: 0, col_span: 1, row_span: 2,
                    paragraphs: vec![para(0, "anchor")],
                    ..TableCell::default()
                },
                in_cell,
                cell(0, 1, "row2"),
            ],
            ..TableControl::default()
        };
        let doc = make_doc(
            vec![style("본문")],
            vec![top_pic, para_with_table(t)],
        );
        let md = to_markdown(&doc);
        let n1 = md.find("{{그림 1.}}").expect("top-level N=1");
        let n2 = md.find("{{그림 2.}}").expect("cell-embedded N=2");
        assert!(n1 < n2, "counter order broken: {md}");
    }

    #[test]
    fn picture_without_caption_keeps_bare_placeholder() {
        use hwp_transpiler_core::ir::PictureControl;

        let mut doc = IrDocument::default();
        doc.sections.push(Section {
            paragraphs: vec![Paragraph {
                controls: vec![Control {
                    kind: ControlKind::Picture(PictureControl {
                        bin_id: 1,
                        width_hwpu: 0,
                        height_hwpu: 0,
                        caption_text: None,
                    }),
                }],
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        let md = to_markdown(&doc);
        assert!(md.contains("{{그림 1.}}"), "got: {md}");
        assert!(!md.contains("{{그림 1. "), "no trailing space-text: {md}");
    }

    #[test]
    fn multi_paragraph_heading_box_still_promotes() {
        // Regression: TRL chapter-7 pattern. HWP broke a long title
        // across two paragraphs in the same cell; the old single-para
        // gate refused to promote, so the heading fell through to the
        // passage path and lost its `##` marker.
        let t = TableControl {
            rows: 1,
            cols: 2,
            row_cell_counts: vec![2],
            cells: vec![
                TableCell {
                    col: 0,
                    row: 0,
                    col_span: 1,
                    row_span: 1,
                    paragraphs: vec![
                        para(0, "7. 연구개발성과의 활용방안 및 기대효과"),
                        para(0, "(기술성·시장성 및 사업성 검토 방안 등)"),
                    ],
                    ..TableCell::default()
                },
                cell(1, 0, ""),
            ],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(t)]);
        let md = to_markdown(&doc);
        assert_eq!(
            md,
            "## 7. 연구개발성과의 활용방안 및 기대효과 (기술성·시장성 및 사업성 검토 방안 등)\n"
        );
    }

    #[test]
    fn multi_paragraph_heading_with_blank_in_between_still_joins() {
        // A stray empty paragraph between the two title lines must not
        // break promotion — filtered out before the space-join.
        let t = TableControl {
            rows: 1, cols: 1, row_cell_counts: vec![1],
            cells: vec![TableCell {
                col: 0, row: 0, col_span: 1, row_span: 1,
                paragraphs: vec![
                    para(0, "1. 기술개발"),
                    para(0, ""),
                    para(0, "목표"),
                ],
                ..TableCell::default()
            }],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(t)]);
        assert_eq!(to_markdown(&doc), "## 1. 기술개발 목표\n");
    }

    #[test]
    fn multi_paragraph_without_numeric_prefix_still_not_a_heading() {
        // Guard against over-promotion: multi-para cell whose joined
        // text has no chapter prefix must stay out of the heading path.
        let t = TableControl {
            rows: 1, cols: 1, row_cell_counts: vec![1],
            cells: vec![TableCell {
                col: 0, row: 0, col_span: 1, row_span: 1,
                paragraphs: vec![
                    para(0, "민관공동기술사업화"),
                    para(0, "연구개발계획서"),
                ],
                ..TableCell::default()
            }],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(t)]);
        let md = to_markdown(&doc);
        assert!(!md.starts_with("## "), "got: {md}");
        assert!(!md.starts_with("### "), "got: {md}");
    }

    #[test]
    fn box_table_with_paren_prefix_becomes_subheading() {
        let t = TableControl {
            rows: 1, cols: 1, row_cell_counts: vec![1],
            cells: vec![cell(0, 0, "(1) 선행연구개발 이력")],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(t)]);
        assert_eq!(to_markdown(&doc), "### (1) 선행연구개발 이력\n");
    }

    #[test]
    fn box_table_with_long_body_stays_a_table() {
        // 1×1 ragged with multi-line body — the "blockquote-like" usage,
        // not a heading. Should NOT collapse to ##.
        let t = TableControl {
            rows: 1, cols: 1, row_cell_counts: vec![1],
            cells: vec![cell(
                0, 0,
                "○ 첫째 항목 ○ 둘째 항목 ○ 셋째 항목 ○ 넷째 항목 ○ 다섯째 항목 더 길게 길게",
            )],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(t)]);
        let md = to_markdown(&doc);
        assert!(!md.starts_with("## "), "long body must not become heading: {md:?}");
    }

    #[test]
    fn box_table_without_numeric_prefix_stays_a_table() {
        let t = TableControl {
            rows: 1, cols: 1, row_cell_counts: vec![1],
            cells: vec![cell(0, 0, "그냥 평범한 짧은 문구")],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(t)]);
        let md = to_markdown(&doc);
        assert!(!md.starts_with("## "), "no prefix must not promote to heading");
        assert!(!md.starts_with("### "));
    }

    #[test]
    fn colspan_only_table_collapses_into_md_grid() {
        // row 0 is one merged 1×3 header; row 1 has three plain cells.
        // GFM has no colspan, so the merge widens the first column with
        // empty siblings — the table still parses correctly downstream.
        let t = TableControl {
            rows: 2,
            cols: 3,
            row_cell_counts: vec![1, 3],
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
        let doc = make_doc(vec![style("본문")], vec![para_with_table(t)]);
        let md = to_markdown(&doc);
        assert!(md.contains("| merged header |  |  |"), "got: {md}");
        assert!(md.contains("| --- | --- | --- |"));
        assert!(md.contains("| x | y | z |"));
    }

    #[test]
    fn rowspan_table_falls_back_to_bullets() {
        // Any row_span > 1 forces the bullet path because GFM cannot
        // express vertical merges.
        let t = TableControl {
            rows: 2,
            cols: 2,
            row_cell_counts: vec![1, 1],
            cells: vec![
                TableCell {
                    col: 0, row: 0, col_span: 1, row_span: 2,
                    paragraphs: vec![para(0, "vmerged")],
                    ..TableCell::default()
                },
                cell(1, 0, "a"),
                cell(1, 1, "b"),
            ],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(t)]);
        let md = to_markdown(&doc);
        assert!(md.contains("<!-- table 2×2"), "got: {md}");
        assert!(md.contains("- [0,0] span 2×1: vmerged"));
    }

    #[test]
    fn consecutive_empty_cells_collapse_into_range() {
        // Gantt-style row: [r,1] has text, [r,2..5] are empty stubs.
        // Forced to bullets via row_span=2 on cell [0,0].
        let t = TableControl {
            rows: 2, cols: 6,
            row_cell_counts: vec![1, 5],
            cells: vec![
                TableCell {
                    col: 0, row: 0, col_span: 1, row_span: 2,
                    paragraphs: vec![para(0, "vmerged")],
                    ..TableCell::default()
                },
                cell(1, 1, "task A"),
                cell(2, 1, ""),
                cell(3, 1, ""),
                cell(4, 1, ""),
                cell(5, 1, ""),
            ],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(t)]);
        let md = to_markdown(&doc);
        assert!(md.contains("- [1,1]: task A"), "got: {md}");
        assert!(md.contains("- [1,2..5]: (empty)"), "got: {md}");
        // The expanded version (4 separate `- [1,c]:` lines) must NOT
        // appear.
        assert!(!md.contains("- [1,2]:\n"));
        assert!(!md.contains("- [1,5]:\n"));
    }

    #[test]
    fn single_empty_cell_does_not_collapse() {
        // Lone empty cell sandwiched between filled cells stays as-is.
        let t = TableControl {
            rows: 2, cols: 3,
            row_cell_counts: vec![1, 3],
            cells: vec![
                TableCell {
                    col: 0, row: 0, col_span: 1, row_span: 2,
                    paragraphs: vec![para(0, "vmerged")],
                    ..TableCell::default()
                },
                cell(1, 1, "left"),
                cell(2, 1, ""),
            ],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(t)]);
        let md = to_markdown(&doc);
        assert!(md.contains("- [1,2]:"), "got: {md}");
        assert!(!md.contains("(empty)"), "single empty must not be range-collapsed: {md}");
    }

    #[test]
    fn multiline_cell_inlines_with_middle_dot() {
        // Forced to bullets by row_span. Cell has 3 paragraphs that
        // previously rendered as 3 separate un-indented lines (breaking
        // out of the list item). Now they collapse to one line joined by
        // ` · `.
        let t = TableControl {
            rows: 2, cols: 2,
            row_cell_counts: vec![1, 1],
            cells: vec![
                TableCell {
                    col: 0, row: 0, col_span: 1, row_span: 2,
                    paragraphs: vec![
                        para(0, "1차"),
                        para(0, "예비"),
                        para(0, "연구"),
                    ],
                    ..TableCell::default()
                },
                cell(1, 0, "x"),
                cell(1, 1, "y"),
            ],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(t)]);
        let md = to_markdown(&doc);
        assert!(md.contains("- [0,0] span 2×1: 1차 · 예비 · 연구"), "got: {md}");
        // Old broken form (raw newlines) must not reappear.
        assert!(!md.contains("- [0,0] span 2×1: 1차\n예비"), "got: {md}");
    }

    #[test]
    fn long_cell_text_breaks_into_sub_bullets() {
        // HWP "section-in-a-box" pattern: a table cell holds many
        // paragraphs of body copy. ` · ` inlining would flatten them into
        // one massive line and erase the structure (○, -, etc.) inside.
        // We force the bullet path here with a row_span=2 sibling so the
        // long-cell heuristic actually runs.
        let long = "이것은 비교적 긴 한 단락의 본문이며 가운데 아주 많은 글자를 가지고 있어서 임계치 200자에 도달하기 위한 충분한 길이를 확보합니다.";
        let t = TableControl {
            rows: 2, cols: 2, row_cell_counts: vec![2, 1],
            cells: vec![
                TableCell {
                    col: 0, row: 0, col_span: 1, row_span: 1,
                    paragraphs: vec![
                        para(0, "○ 첫 번째 단락"),
                        para(0, long),
                        para(0, long),
                        para(0, long),
                        para(0, long),
                        para(0, "○ 마지막 단락"),
                    ],
                    ..TableCell::default()
                },
                // row_span=2 sibling forces the whole table to bullets.
                TableCell {
                    col: 1, row: 0, col_span: 1, row_span: 2,
                    paragraphs: vec![para(0, "side")],
                    ..TableCell::default()
                },
                cell(0, 1, "next"),
            ],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(t)]);
        let md = to_markdown(&doc);
        assert!(md.contains("- [0,0]:\n"), "got: {md}");
        assert!(md.contains("  - ○ 첫 번째 단락\n"), "got: {md}");
        assert!(md.contains("  - ○ 마지막 단락\n"), "got: {md}");
        // The inline form (` · `-joined into one line) must not appear.
        assert!(
            !md.contains("- [0,0]: ○ 첫 번째"),
            "long passage must not be inlined: {md}"
        );
    }

    #[test]
    fn empty_1x1_wrapper_table_is_unwrapped() {
        // Outer 1×1 with no body text + one nested table. The wrapper
        // would normally print `- [0,0]:` and indent everything below it
        // by one level — pure visual noise. Strip it.
        let inner = TableControl {
            rows: 1, cols: 1, row_cell_counts: vec![1],
            cells: vec![cell(0, 0, "real content")],
            ..TableControl::default()
        };
        let outer = TableControl {
            rows: 1, cols: 1, row_cell_counts: vec![1],
            cells: vec![TableCell {
                col: 0, row: 0, col_span: 1, row_span: 1,
                paragraphs: vec![Paragraph {
                    controls: vec![Control {
                        kind: ControlKind::Table(inner),
                    }],
                    ..Paragraph::default()
                }],
                ..TableCell::default()
            }],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(outer)]);
        let md = to_markdown(&doc);
        // Inner content surfaces at the top level — no `[0,0]:` wrapper
        // line, no extra indent. (After unwrap the inner 1×1 then takes
        // the passage path and renders as plain prose.)
        assert!(md.contains("real content"), "got: {md}");
        assert!(!md.contains("- [0,0]:"), "wrapper must be stripped: {md}");
    }

    #[test]
    fn wrapper_with_text_is_not_unwrapped() {
        // 1×1 cell that has its own body text alongside a nested table —
        // the text would be lost if we unwrapped, so keep the wrapper.
        let inner = TableControl {
            rows: 1, cols: 1, row_cell_counts: vec![1],
            cells: vec![cell(0, 0, "inner")],
            ..TableControl::default()
        };
        let outer = TableControl {
            rows: 1, cols: 1, row_cell_counts: vec![1],
            cells: vec![TableCell {
                col: 0, row: 0, col_span: 1, row_span: 1,
                paragraphs: vec![
                    para(0, "outer caption"),
                    Paragraph {
                        controls: vec![Control {
                            kind: ControlKind::Table(inner),
                        }],
                        ..Paragraph::default()
                    },
                ],
                ..TableCell::default()
            }],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(outer)]);
        let md = to_markdown(&doc);
        assert!(md.contains("outer caption"), "outer text must survive: {md}");
        assert!(md.contains("inner"));
    }

    #[test]
    fn single_row_box_without_prefix_becomes_passage() {
        // Doc-title pattern: 1×1 frame holding a long run-on title.
        // No `1. `/`(1) ` prefix → not a heading, but rendering it as a
        // 1-cell MD table would produce `| ... |\n| --- |` which reads
        // badly. Emit as plain text instead.
        let t = TableControl {
            rows: 1, cols: 1, row_cell_counts: vec![1],
            cells: vec![cell(0, 0, "민관공동기술사업화 연구개발계획서 1단계")],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(t)]);
        let md = to_markdown(&doc);
        assert_eq!(md, "민관공동기술사업화 연구개발계획서 1단계\n");
    }

    #[test]
    fn multi_paragraph_passage_joins_with_spaces() {
        // HWP splits long titles across paragraphs for line wrapping. The
        // joined result must still go through the passage path (joined
        // length is ~45 chars, under the 100-char limit).
        let t = TableControl {
            rows: 1, cols: 1, row_cell_counts: vec![1],
            cells: vec![TableCell {
                col: 0, row: 0, col_span: 1, row_span: 1,
                paragraphs: vec![
                    para(0, "민관공동기술사업화 연구개발계획서"),
                    para(0, "1단계 (PoC·PoM)"),
                ],
                ..TableCell::default()
            }],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(t)]);
        let md = to_markdown(&doc);
        assert_eq!(md, "민관공동기술사업화 연구개발계획서 1단계 (PoC·PoM)\n");
    }

    #[test]
    fn single_row_with_empty_companion_becomes_passage() {
        // Same pattern but the box was authored as 1×2 with a blank
        // sibling cell — equally common as a layout trick.
        let t = TableControl {
            rows: 1, cols: 2, row_cell_counts: vec![2],
            cells: vec![cell(0, 0, "문서 제목"), cell(1, 0, "")],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(t)]);
        let md = to_markdown(&doc);
        assert_eq!(md, "문서 제목\n");
    }

    #[test]
    fn multi_row_table_is_not_a_passage() {
        // Real 2×2 data table — must stay as a table, not collapse into
        // its first non-empty cell's text.
        let t = TableControl {
            rows: 2, cols: 2, row_cell_counts: vec![2, 2],
            cells: vec![
                cell(0, 0, "header"),
                cell(1, 0, ""),
                cell(0, 1, "left"),
                cell(1, 1, "right"),
            ],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(t)]);
        let md = to_markdown(&doc);
        assert!(md.contains("| header"), "got: {md}");
        assert!(md.contains("| --- "), "got: {md}");
    }

    #[test]
    fn ragged_table_with_holes_falls_back_to_bullets() {
        // 2×2 declared but only 2 cells exist (row 1 is missing entirely).
        // Without a way to express "no cell here", we must NOT fabricate
        // empty rows in the MD grid — fall back to bullets so the gap is
        // visible to the reader.
        let t = TableControl {
            rows: 2, cols: 2,
            row_cell_counts: vec![2, 0],
            cells: vec![cell(0, 0, "a"), cell(1, 0, "b")],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(t)]);
        let md = to_markdown(&doc);
        assert!(md.contains("<!-- table 2×2"));
    }

    #[test]
    fn cell_pipe_is_escaped() {
        // 2×1 — must take the MD-grid path (not the 1-row passage one),
        // so we can verify pipe escaping inside an actual table cell.
        let t = TableControl {
            rows: 2,
            cols: 1,
            row_cell_counts: vec![1, 1],
            cells: vec![cell(0, 0, "a|b"), cell(0, 1, "x")],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(t)]);
        let md = to_markdown(&doc);
        assert!(md.contains("| a\\|b |"), "got: {md}");
    }
}
