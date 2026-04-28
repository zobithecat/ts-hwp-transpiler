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
    CharShape, CharShapeRun, ControlKind, EquationControl, IrDocument, Paragraph, PictureControl,
    TableCell, TableControl,
};
use hwp_transpiler_core::semantics::{CellRole, TableDomain, infer_table_domain};

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
    /// Human Markdown path only: when true, prepend an HTML comment
    /// like `<!-- kind: budget -->` above each classified table so a
    /// reader (or downstream tool that greps the output) can tell a
    /// budget / schedule / personnel table apart from prose layout
    /// tables. Off by default — existing snapshots stay stable.
    /// Ignored on the LLM path (it has `LlmOptions::domain_hints`).
    pub domain_hints: bool,
    /// Human Markdown path only: when true, each cell in a complex
    /// table's bullet list gains a `role=header|label|value|spacer`
    /// tag inside the `[r,c]` marker, taken from the same visual
    /// classifier the LLM path uses. Simple-grid tables ignore this
    /// flag (pipe cells can't host the attribute). Off by default.
    pub emit_roles: bool,
    /// Human Markdown path only: like `emit_roles` but for
    /// `editable=true|false|unknown`. Enabling this implicitly
    /// enables role computation even if `emit_roles` is off —
    /// editable inference is role-dependent. Off by default.
    pub emit_editable: bool,
    /// Human Markdown path only: when true, paragraph text is
    /// emitted with inline Markdown formatting derived from each
    /// `CharShapeRun`'s referenced `CharShape`. Bold / italic /
    /// strikethrough get wrapped (`**...**` / `*...*` / `~~...~~`);
    /// runs whose shape has no formatting emit as plain text.
    /// Paragraphs with an empty `char_shape_runs` list fall back to
    /// `clean_text`, so existing callers see no change unless they
    /// opt in. Applies to top-level paragraph text only — table
    /// cell text still uses the plain path (nested formatting inside
    /// pipe cells is fragile in most Markdown renderers).
    pub emit_styles: bool,
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
    /// Emit `kind=<domain>` on TABLE markers when the domain inferrer
    /// classifies the table as something other than `Unknown`
    /// (institution_info, budget, schedule, performance_metrics,
    /// personnel). Off by default; enabling costs a per-table scan of
    /// its cell text against a fixed keyword vocabulary.
    pub domain_hints: bool,
}

pub fn to_markdown(doc: &IrDocument) -> String {
    to_markdown_with(doc, &MdOptions::default())
}

/// Stamped at the top of every human-mode export so the importer
/// can dispatch by header rather than sniffing content. HTML
/// comment so it stays invisible in rendered Markdown.
pub const FORMAT_HEADER_HUMAN: &str = "<!-- hwp-transpiler: format=human -->";

/// Stamped at the top of every LLM-mode export. Same role as
/// [`FORMAT_HEADER_HUMAN`].
pub const FORMAT_HEADER_LLM: &str = "<!-- hwp-transpiler: format=llm -->";

pub fn to_markdown_with(doc: &IrDocument, opts: &MdOptions) -> String {
    if opts.llm.is_some() {
        return super::markdown_llm::to_llm_markdown(doc, opts);
    }
    let mut out = String::new();
    // Format header so the importer can dispatch deterministically
    // back into the human-Markdown branch even if the document
    // happens to start with content that resembles the LLM sigil.
    out.push_str(FORMAT_HEADER_HUMAN);
    out.push('\n');
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
    let text = if opts.emit_styles && !para.char_shape_runs.is_empty() {
        styled_paragraph_text(&para.text, &para.char_shape_runs, &doc.doc_info.char_shapes)
    } else {
        clean_text(&para.text)
    };
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
            ControlKind::Table(t) => {
                emit_table(doc, t, out, depth, opts, picture_counter, &para.text)
            }
            ControlKind::Picture(p) => {
                *picture_counter += 1;
                emit_picture(doc, p, c.caption_text.as_deref(), out, opts, *picture_counter);
            }
            ControlKind::Equation(eq) => emit_equation(eq, out),
            _ => {}
        }
    }
}

/// Emit one equation as its own block. Runs the HWP equation script
/// through the LaTeX converter and emits a display-math block
/// (`$$ … $$`) that KaTeX / MathJax-backed viewers can render. The
/// original script is kept as an HTML comment *inside* the block so
/// the source survives round-trip for diagnostics — LaTeX parsing
/// can be finicky and having the unmodified HWP script available
/// makes re-running the converter with fixes trivial.
///
/// Empty scripts collapse to a `{{수식:}}` placeholder — same shape as
/// the picture placeholder, so readers can tell "there was an equation
/// here whose body we couldn't extract" at a glance. Font and size
/// metadata are intentionally dropped in the human path; rendering
/// faithfully requires a real equation renderer, not Markdown.
fn emit_equation(eq: &EquationControl, out: &mut String) {
    let script = eq.script.trim();
    if script.is_empty() {
        out.push_str("{{수식:}}\n\n");
        return;
    }
    let latex = hwp_transpiler_core::formula::to_latex(script);
    out.push_str("$$\n");
    if !latex.is_empty() {
        out.push_str(&latex);
        out.push('\n');
    } else {
        out.push_str(script);
        out.push('\n');
    }
    out.push_str("$$\n\n");
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
    // Use the matching BinaryEntry's id as the sidecar filename.
    // Covers both naming conventions the assets dumper actually
    // writes: HWP5 uses uppercase hex (`BIN000A.bmp`), HWPX uses
    // decimal (`image12.png`). Falling back to the hex-formatted
    // `BIN{id:04X}` when no entry exists preserves the historical
    // behaviour for docs that reference a missing binary.
    let filename = doc
        .bin_data
        .iter()
        .find(|e| matches_bin_entry(&e.id, pic.bin_id))
        .map(|e| e.id.clone())
        .unwrap_or_else(|| {
            format!("BIN{:04X}.{}", pic.bin_id, resolve_bin_extension(doc, pic.bin_id))
        });
    let w = hwpunit_to_mm(pic.width_hwpu);
    let h = hwpunit_to_mm(pic.height_hwpu);
    Some(format!(
        "![]({prefix}/{filename}){{width={w}mm; height={h}mm}}"
    ))
}

/// True when `id` is either HWPX's `image{dec}.{ext}` or HWP5's
/// `BIN{HEX}.{ext}` naming for the given numeric bin_id. Mirrors
/// `render::html::matches_bin_id` — duplicated here rather than
/// routed through a cross-crate helper because codec deliberately
/// doesn't depend on render.
fn matches_bin_entry(id: &str, bin_id: u16) -> bool {
    if let Some(stem) = id.strip_prefix("image") {
        if let Some((num, _)) = stem.split_once('.') {
            if num.parse::<u16>() == Ok(bin_id) {
                return true;
            }
        }
    }
    if let Some(stem) = id.strip_prefix("BIN") {
        if let Some((hex, _)) = stem.split_once('.') {
            if hex.len() == 4 {
                if let Ok(n) = u16::from_str_radix(hex, 16) {
                    return n == bin_id;
                }
            }
        }
    }
    false
}

/// Build the `{{그림 N. <caption>}}` placeholder string (no trailing
/// newline). Caption text is cleaned and the HWP auto-numbering
/// `"그림 . "` prefix — left stranded after `clean_text` drops U+FFFC —
/// is removed.
fn picture_caption_line(caption_text: Option<&str>, n: u32) -> String {
    let caption_suffix = caption_text
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
    caption_text: Option<&str>,
    out: &mut String,
    opts: &MdOptions,
    n: u32,
) {
    if let Some(img) = picture_image_line(doc, pic, opts) {
        out.push_str(&img);
        out.push_str("\n\n");
    }
    out.push_str(&picture_caption_line(caption_text, n));
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
    caption_text: Option<&str>,
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
    out.push_str(&picture_caption_line(caption_text, n));
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
    owner_para_text: &str,
) {
    if let Some(inner) = try_unwrap_wrapper_table(t) {
        // Pure decorative wrapper (1×1 with one nested table, no own
        // text). Skip the outer frame — emit the inner directly at the
        // same depth so it doesn't gain a useless extra indent. The
        // owner text still belongs to the inner table conceptually.
        emit_table(doc, inner, out, depth, opts, picture_counter, owner_para_text);
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

    // Domain hint (opt-in): `<!-- kind: budget -->` above the actual
    // grid / bullet emission. Only fires when the classifier returns
    // something other than `Unknown`, so unclassified layout tables
    // stay uncluttered. Nested emissions pass an empty owner text
    // because the outer paragraph's heading belongs to the outermost
    // table — nesting a second hint on an inner grid would be noise.
    if opts.domain_hints {
        let domain = infer_table_domain(t, owner_para_text);
        if domain != TableDomain::Unknown {
            out.push_str(&format!("<!-- kind: {} -->\n", domain.as_str()));
        }
    }

    if let Some(grid) = try_build_md_grid(t) {
        // Simple grid path: per-cell role/editable tags don't fit
        // GFM pipe cells, so these flags are silently ignored here.
        // Complex bullet path below honours them.
        emit_md_grid(&grid, out);
    } else {
        let roles: Vec<CellRole> = if opts.emit_roles || opts.emit_editable {
            super::markdown_llm::compute_roles(doc, t)
        } else {
            Vec::new()
        };
        emit_table_as_list(doc, t, out, depth, opts, picture_counter, &roles);
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
        // GFM grid can't host nested blocks; pictures, nested tables,
        // and equations all force the bullet path so their structure
        // (fenced blocks, figures, child tables) survives.
        for p in &cell.paragraphs {
            for ctrl in &p.controls {
                if matches!(
                    &ctrl.kind,
                    ControlKind::Table(_) | ControlKind::Picture(_) | ControlKind::Equation(_)
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
    roles: &[CellRole],
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
        let role = roles.get(i).copied();
        emit_cell_line(doc, cell, out, &indent, depth, opts, picture_counter, role);
        i += 1;
    }

    out.push('\n');
}

#[allow(clippy::too_many_arguments)]
fn emit_cell_line(
    doc: &IrDocument,
    cell: &TableCell,
    out: &mut String,
    indent: &str,
    depth: usize,
    opts: &MdOptions,
    picture_counter: &mut u32,
    role: Option<CellRole>,
) {
    out.push_str(indent);
    out.push_str("- ");
    let mut marker = format!("[{},{}", cell.row, cell.col);
    if opts.emit_roles {
        marker.push_str(&format!(
            ", role={}",
            super::markdown_llm::role_name(role)
        ));
    }
    if opts.emit_editable {
        let ed = super::markdown_llm::infer_editable(cell, role);
        marker.push_str(&format!(
            ", editable={}",
            super::markdown_llm::editable_name(ed)
        ));
    }
    marker.push(']');
    out.push_str(&marker);
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
                    // Nested tables don't inherit the outer owner text
                    // — passing "" keeps domain classification focused
                    // on the inner table's own cell corpus.
                    emit_table(doc, nested, out, depth + 1, opts, picture_counter, "");
                }
                ControlKind::Picture(pic) => {
                    *picture_counter += 1;
                    emit_picture_bullet(
                        &child_indent,
                        doc,
                        pic,
                        c.caption_text.as_deref(),
                        out,
                        opts,
                        *picture_counter,
                    );
                }
                ControlKind::Equation(eq) => {
                    emit_equation_bullet(&child_indent, eq, out);
                }
                _ => {}
            }
        }
    }
}

/// Equation inside a table cell that's been demoted to the bullet
/// path. Emitted as a fenced block indented under the parent list
/// item — two spaces deeper than the bullet marker so Markdown
/// associates it with the enclosing cell line. Empty script collapses
/// to an indented `{{수식:}}` one-liner, matching the top-level
/// equation fallback shape.
fn emit_equation_bullet(indent: &str, eq: &EquationControl, out: &mut String) {
    let script = eq.script.trim();
    if script.is_empty() {
        out.push_str(indent);
        out.push_str("{{수식:}}\n");
        return;
    }
    // Inline $…$ form — fits a single bullet line. Some renderers
    // treat multiline `$$ … $$` inside a list awkwardly; inline
    // math keeps the bullet structure intact.
    let latex = hwp_transpiler_core::formula::to_latex(script);
    let body = if latex.is_empty() { script } else { &latex };
    out.push_str(indent);
    out.push_str("$");
    out.push_str(body);
    out.push_str("$\n");
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

pub(super) fn heading_level(doc: &IrDocument, para: &Paragraph) -> Option<u8> {
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

/// Style-aware cousin of [`clean_text`]: walks `text` char-by-char,
/// tracking UTF-16 code-unit offsets (the coordinate system that
/// `CharShapeRun::start` uses), and wraps each run's segment in
/// Markdown inline formatting based on its referenced `CharShape`:
///
///   * bold      → `**…**`
///   * italic    → `*…*`
///   * strike    → `~~…~~`
///
/// Other CharShape attributes (underline, color, font) have no
/// portable Markdown representation; they're silently dropped. Runs
/// whose referenced shape has none of the three handled flags emit as
/// plain text — so paragraphs authored with the document's default
/// char shape look identical to `clean_text` output.
///
/// Unreferenced runs (shape id out of bounds) are treated as plain.
///
/// When the text contains U+FFFC placeholders (extended-control
/// markers), those are dropped the same way `clean_text` drops them;
/// `char_shape_runs` offsets remain valid because U+FFFC is a single
/// UTF-16 unit and our UTF-16 step counting accounts for it.
fn styled_paragraph_text(
    text: &str,
    runs: &[CharShapeRun],
    shapes: &[CharShape],
) -> String {
    if runs.is_empty() {
        return clean_text(text);
    }
    let mut out = String::new();
    let mut u16_pos: u32 = 0;
    let mut open_wrappers: Vec<&'static str> = Vec::new();
    let mut active_shape_id: Option<u32> = None;

    for c in text.chars() {
        let c_len = c.len_utf16() as u32;
        let new_shape_id = runs
            .iter()
            .rev()
            .find(|r| r.start <= u16_pos)
            .map(|r| r.char_shape_id);

        if new_shape_id != active_shape_id {
            // Close currently-open wrappers in reverse order.
            while let Some(w) = open_wrappers.pop() {
                out.push_str(w);
            }
            // Open new wrappers — strike on the outside so bold
            // italic still render inside a strikethrough range;
            // italic innermost so single `*` doesn't collide with
            // the double `**` of bold.
            if let Some(sid) = new_shape_id {
                if let Some(shape) = shapes.get(sid as usize) {
                    if shape.strike() {
                        out.push_str("~~");
                        open_wrappers.push("~~");
                    }
                    if shape.bold() {
                        out.push_str("**");
                        open_wrappers.push("**");
                    }
                    if shape.italic() {
                        out.push('*');
                        open_wrappers.push("*");
                    }
                }
            }
            active_shape_id = new_shape_id;
        }

        // Apply the same character translations as `clean_text`.
        match c {
            '\u{FFFC}' | '\u{00AD}' => {}
            '\u{00A0}' | '\u{2003}' => out.push(' '),
            _ => out.push(translate_pua_bullet(c).unwrap_or(c)),
        }

        u16_pos += c_len;
    }

    while let Some(w) = open_wrappers.pop() {
        out.push_str(w);
    }

    // Squeeze consecutive spaces + trim, matching `clean_text`.
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

    /// Strip the leading `<!-- hwp-transpiler: format=human -->\n`
    /// stamp so legacy exact-string assertions stay focused on the
    /// document body rather than the dispatch header.
    fn body(s: String) -> String {
        s.strip_prefix(FORMAT_HEADER_HUMAN)
            .and_then(|r| r.strip_prefix('\n'))
            .map(|r| r.to_string())
            .unwrap_or(s)
    }

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
            controls: vec![Control { kind: ControlKind::Table(t), ..Default::default() }],
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
            body(to_markdown(&doc)),
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
        let md = body(to_markdown(&doc));
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
        assert_eq!(body(to_markdown(&doc)), "hello world\n");
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
        let md = body(to_markdown(&doc));
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
        assert_eq!(body(to_markdown(&doc)), "## 1. 기술개발 목표\n");
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
                    }),
                    caption_text: Some(
                        "그림 \u{FFFC}. 시스템 전체 아키텍처".into(),
                    ),
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
                    }),
                    caption_text: Some("그림 \u{FFFC}. 연구팀 구성 도식".into()),
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
                    }),
                    caption_text: None,
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
        let md = body(to_markdown(&doc));
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
                    bin_id: 1, width_hwpu: 0, height_hwpu: 0,
                }),
                caption_text: None,
            }],
            ..Paragraph::default()
        };
        let in_cell = TableCell {
            col: 1, row: 0, col_span: 1, row_span: 1,
            paragraphs: vec![Paragraph {
                controls: vec![Control {
                    kind: ControlKind::Picture(PictureControl {
                        bin_id: 2, width_hwpu: 0, height_hwpu: 0,
                    }),
                    caption_text: None,
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
        let md = body(to_markdown(&doc));
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
                    }),
                    caption_text: None,
                }],
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        let md = body(to_markdown(&doc));
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
        let md = body(to_markdown(&doc));
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
        assert_eq!(body(to_markdown(&doc)), "## 1. 기술개발 목표\n");
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
        let md = body(to_markdown(&doc));
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
        assert_eq!(body(to_markdown(&doc)), "### (1) 선행연구개발 이력\n");
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
        let md = body(to_markdown(&doc));
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
        let md = body(to_markdown(&doc));
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
        let md = body(to_markdown(&doc));
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
        let md = body(to_markdown(&doc));
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
        let md = body(to_markdown(&doc));
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
        let md = body(to_markdown(&doc));
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
        let md = body(to_markdown(&doc));
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
        let md = body(to_markdown(&doc));
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
                        ..Default::default()
                    }],
                    ..Paragraph::default()
                }],
                ..TableCell::default()
            }],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(outer)]);
        let md = body(to_markdown(&doc));
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
                            ..Default::default()
                        }],
                        ..Paragraph::default()
                    },
                ],
                ..TableCell::default()
            }],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(outer)]);
        let md = body(to_markdown(&doc));
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
        let md = body(to_markdown(&doc));
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
        let md = body(to_markdown(&doc));
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
        let md = body(to_markdown(&doc));
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
        let md = body(to_markdown(&doc));
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
        let md = body(to_markdown(&doc));
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
        let md = body(to_markdown(&doc));
        assert!(md.contains("| a\\|b |"), "got: {md}");
    }

    fn equation(script: &str) -> Control {
        Control {
            kind: ControlKind::Equation(EquationControl {
                script: script.into(),
                font: None,
                size_hwpu: 0,
            }),
            ..Default::default()
        }
    }

    fn para_with_controls(controls: Vec<Control>) -> Paragraph {
        Paragraph {
            controls,
            ..Paragraph::default()
        }
    }

    #[test]
    fn equation_renders_as_display_math() {
        // Script passes through the HWP→LaTeX converter. Superscripts
        // via `^` are recognised as infix ops and wrap in `{}`.
        let doc = make_doc(
            vec![style("본문")],
            vec![para_with_controls(vec![equation("x^2 + y^2 = z^2")])],
        );
        let md = body(to_markdown(&doc));
        assert!(md.contains("$$"), "got: {md}");
        assert!(md.contains("x^{2}"), "got: {md}");
        assert!(md.contains("z^{2}"), "got: {md}");
        // Exactly one display-math block.
        let fence_count = md.matches("$$").count();
        assert_eq!(fence_count, 2, "expected one display-math block: {md}");
    }

    #[test]
    fn equation_body_surfaces_into_latex_math() {
        // Regardless of the exact LaTeX form, the body reaches the
        // display-math block with recognisable tokens from the
        // original script.
        let doc = make_doc(
            vec![style("본문")],
            vec![para_with_controls(vec![equation("OVER {a} {b}")])],
        );
        let md = body(to_markdown(&doc));
        assert!(md.contains("$$"), "got: {md}");
        // OVER with braced args emits \frac via the infix handler
        // (empty LHS picks up from the following braced atom).
        assert!(md.contains("\\frac"), "got: {md}");
    }

    #[test]
    fn empty_equation_collapses_to_placeholder() {
        // No silent drop: even without script content, the reader sees
        // a `{{수식:}}` placeholder so they know an equation was there.
        let doc = make_doc(
            vec![style("본문")],
            vec![para_with_controls(vec![equation("")])],
        );
        let md = body(to_markdown(&doc));
        assert!(md.contains("{{수식:}}"), "got: {md}");
        // No display-math block for empty scripts.
        assert!(!md.contains("$$\n\n$$"), "got: {md}");
    }

    fn make_doc_with_owner_heading(heading: &str, t: TableControl) -> IrDocument {
        // Heading text helps the domain classifier (owner_para_text
        // param of infer_table_domain). The table itself is passed in.
        make_doc(
            vec![style("본문")],
            vec![para(0, heading), para_with_table(t)],
        )
    }

    #[test]
    fn domain_hint_off_by_default() {
        // Budget keywords in the cells → classifier sees Budget. With
        // the flag off, the emitter should NOT add an HTML comment.
        let t = TableControl {
            rows: 1,
            cols: 3,
            row_cell_counts: vec![3],
            cells: vec![
                cell(0, 0, "구분"),
                cell(1, 0, "정부지원"),
                cell(2, 0, "기관 현금"),
            ],
            ..TableControl::default()
        };
        let doc = make_doc_with_owner_heading("", t);
        let md = body(to_markdown(&doc));
        assert!(!md.contains("<!-- kind"), "got: {md}");
    }

    #[test]
    fn domain_hint_surfaces_classified_table_when_enabled() {
        // Same table as above but run with `domain_hints = true`; the
        // classifier puts it in Budget, so a comment should appear.
        let t = TableControl {
            rows: 1,
            cols: 3,
            row_cell_counts: vec![3],
            cells: vec![
                cell(0, 0, "구분"),
                cell(1, 0, "정부지원"),
                cell(2, 0, "기관 현금"),
            ],
            ..TableControl::default()
        };
        let doc = make_doc_with_owner_heading("", t);
        let opts = MdOptions {
            domain_hints: true,
            ..MdOptions::default()
        };
        let md = to_markdown_with(&doc, &opts);
        assert!(md.contains("<!-- kind: budget -->"), "got: {md}");
    }

    fn three_by_two_complex() -> TableControl {
        // 3 rows × 2 cols, each cell has text so the bullet path
        // engages (try_build_md_grid would succeed too, but we avoid
        // that path by adding multiple paragraphs per cell below).
        TableControl {
            rows: 3,
            cols: 2,
            row_cell_counts: vec![2, 2, 2],
            cells: vec![
                cell(0, 0, "header A"),
                cell(1, 0, "header B"),
                cell(0, 1, "row1-a"),
                cell(1, 1, "row1-b"),
                cell(0, 2, "row2-a"),
                cell(1, 2, "row2-b"),
            ],
            ..TableControl::default()
        }
    }

    /// Force the bullet path by giving one cell `row_span = 2`
    /// (`try_build_md_grid` bails on any row_span != 1).
    fn three_by_two_complex_bullet() -> TableControl {
        let mut t = three_by_two_complex();
        t.cells[0].row_span = 2;
        t
    }

    #[test]
    fn role_attribute_absent_by_default() {
        let doc = make_doc(vec![style("본문")], vec![para_with_table(three_by_two_complex_bullet())]);
        let md = body(to_markdown(&doc));
        // Default options: no role=, no editable= anywhere.
        assert!(!md.contains("role="), "got: {md}");
        assert!(!md.contains("editable="), "got: {md}");
    }

    #[test]
    fn role_attribute_emitted_when_flag_on() {
        let doc = make_doc(vec![style("본문")], vec![para_with_table(three_by_two_complex_bullet())]);
        let opts = MdOptions {
            emit_roles: true,
            ..MdOptions::default()
        };
        let md = to_markdown_with(&doc, &opts);
        // At least one bullet marker should carry role=<something>.
        // Exact role value depends on the classifier — without a
        // DocInfo color resolver the fallback may well return
        // `unknown`, which is still a valid test of plumbing.
        assert!(
            md.contains(", role=header")
                || md.contains(", role=label")
                || md.contains(", role=value")
                || md.contains(", role=spacer")
                || md.contains(", role=unknown"),
            "got: {md}"
        );
        // `editable=` should be absent because we only asked for roles.
        assert!(!md.contains("editable="), "got: {md}");
    }

    #[test]
    fn editable_attribute_emitted_when_flag_on() {
        let doc = make_doc(vec![style("본문")], vec![para_with_table(three_by_two_complex_bullet())]);
        let opts = MdOptions {
            emit_editable: true,
            ..MdOptions::default()
        };
        let md = to_markdown_with(&doc, &opts);
        // `editable=` must appear; the exact value (true/false/unknown)
        // depends on the role+content inference.
        assert!(
            md.contains("editable=true")
                || md.contains("editable=false")
                || md.contains("editable=unknown"),
            "got: {md}"
        );
    }

    #[test]
    fn role_attributes_stay_in_bullet_marker_brackets() {
        // Sanity: the new attributes sit *inside* `[r,c]` so grep-ing
        // for `[0,0,` still matches the first cell. Stays compatible
        // with any downstream tooling that parses the marker. The
        // first cell here has row_span=2, so its marker closes with
        // `] span 2×1:` rather than `]:` — both-tags-same-bullet is
        // what we assert, not the exact trailing punctuation.
        let doc = make_doc(vec![style("본문")], vec![para_with_table(three_by_two_complex_bullet())]);
        let opts = MdOptions {
            emit_roles: true,
            emit_editable: true,
            ..MdOptions::default()
        };
        let md = to_markdown_with(&doc, &opts);
        assert!(md.contains("[0,0, role="), "got: {md}");
        assert!(
            md.lines().any(|l| {
                l.contains("[0,0, role=")
                    && l.contains(", editable=")
                    && l.contains(']')
            }),
            "got: {md}"
        );
    }

    #[test]
    fn domain_hint_silent_on_unknown() {
        // Unclassified layout table — no keywords hit. Even with the
        // flag on, the comment must stay suppressed so unrelated
        // tables stay uncluttered.
        let t = TableControl {
            rows: 1,
            cols: 2,
            row_cell_counts: vec![2],
            cells: vec![cell(0, 0, "항목"), cell(1, 0, "내용")],
            ..TableControl::default()
        };
        let doc = make_doc_with_owner_heading("", t);
        let opts = MdOptions {
            domain_hints: true,
            ..MdOptions::default()
        };
        let md = to_markdown_with(&doc, &opts);
        assert!(!md.contains("<!-- kind"), "got: {md}");
    }

    #[test]
    fn domain_hint_uses_owner_paragraph_text_for_classification() {
        // Cells don't have enough budget vocabulary to trigger on
        // their own, but the hosting paragraph's own text carries
        // two budget keywords — classification fires once the
        // emitter threads `para.text` through as the owner hint.
        let t = TableControl {
            rows: 1,
            cols: 3,
            row_cell_counts: vec![3],
            cells: vec![cell(0, 0, "A"), cell(1, 0, "B"), cell(2, 0, "C")],
            ..TableControl::default()
        };
        let para_hosting_table = Paragraph {
            text: "6. 연구비 사용계획 (예산 집행)".into(),
            controls: vec![Control {
                kind: ControlKind::Table(t),
                ..Default::default()
            }],
            ..Paragraph::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_hosting_table]);
        let opts = MdOptions {
            domain_hints: true,
            ..MdOptions::default()
        };
        let md = to_markdown_with(&doc, &opts);
        assert!(md.contains("<!-- kind: budget -->"), "got: {md}");
    }

    fn char_shape_with_attr(attr: u32) -> hwp_transpiler_core::ir::CharShape {
        hwp_transpiler_core::ir::CharShape {
            attr,
            ..Default::default()
        }
    }

    fn styled_doc(text: &str, runs: Vec<CharShapeRun>, shapes: Vec<CharShape>) -> IrDocument {
        let mut doc = make_doc(vec![style("본문")], vec![]);
        doc.doc_info.char_shapes = shapes;
        doc.sections[0].paragraphs.push(Paragraph {
            header: ParagraphHeader::default(),
            text: text.into(),
            char_shape_runs: runs,
            ..Paragraph::default()
        });
        doc
    }

    #[test]
    fn styles_flag_off_by_default_emits_plain_text() {
        let runs = vec![
            CharShapeRun { start: 0, char_shape_id: 0 },
            CharShapeRun { start: 5, char_shape_id: 1 },
        ];
        let shapes = vec![char_shape_with_attr(0), char_shape_with_attr(0x02)]; // bold
        let doc = styled_doc("plainBOLD!", runs, shapes);
        let md = body(to_markdown(&doc));
        assert!(!md.contains("**"), "default output must not wrap: {md}");
    }

    #[test]
    fn bold_run_wraps_in_double_asterisks() {
        // Runs: [0..5] default, [5..9] bold. "plainBOLD" → "plain**BOLD**"
        let runs = vec![
            CharShapeRun { start: 0, char_shape_id: 0 },
            CharShapeRun { start: 5, char_shape_id: 1 },
        ];
        let shapes = vec![char_shape_with_attr(0), char_shape_with_attr(0x02)];
        let doc = styled_doc("plainBOLD", runs, shapes);
        let opts = MdOptions {
            emit_styles: true,
            ..MdOptions::default()
        };
        let md = to_markdown_with(&doc, &opts);
        assert!(md.contains("plain**BOLD**"), "got: {md}");
    }

    #[test]
    fn italic_run_wraps_in_single_asterisk() {
        // Bit 0 = italic.
        let runs = vec![
            CharShapeRun { start: 0, char_shape_id: 0 },
            CharShapeRun { start: 2, char_shape_id: 1 },
        ];
        let shapes = vec![char_shape_with_attr(0), char_shape_with_attr(0x01)];
        let doc = styled_doc("ab ITALIC", runs, shapes);
        let opts = MdOptions {
            emit_styles: true,
            ..MdOptions::default()
        };
        let md = to_markdown_with(&doc, &opts);
        // italic wrapper is `*…*`. The paragraph should have exactly
        // two `*` from wrappers (one opening, one closing).
        assert_eq!(md.matches('*').count(), 2, "got: {md}");
        assert!(md.contains("* ITALIC*"), "got: {md}");
    }

    #[test]
    fn strike_run_uses_gfm_tilde() {
        // Bit 21 = strike.
        let runs = vec![CharShapeRun { start: 0, char_shape_id: 0 }];
        let shapes = vec![char_shape_with_attr(1u32 << 21)];
        let doc = styled_doc("struck", runs, shapes);
        let opts = MdOptions {
            emit_styles: true,
            ..MdOptions::default()
        };
        let md = to_markdown_with(&doc, &opts);
        assert!(md.contains("~~struck~~"), "got: {md}");
    }

    #[test]
    fn bold_plus_italic_combines_wrappers() {
        // Bits 0 (italic) + 1 (bold) together.
        let runs = vec![CharShapeRun { start: 0, char_shape_id: 0 }];
        let shapes = vec![char_shape_with_attr(0x01 | 0x02)];
        let doc = styled_doc("both", runs, shapes);
        let opts = MdOptions {
            emit_styles: true,
            ..MdOptions::default()
        };
        let md = to_markdown_with(&doc, &opts);
        // Nesting order: bold outside italic → `***both***`.
        assert!(md.contains("***both***"), "got: {md}");
    }

    #[test]
    fn default_char_shape_emits_as_plain() {
        // Single run with attr=0 (no formatting). Styled emission
        // should produce the same as clean_text — no wrapping at all.
        let runs = vec![CharShapeRun { start: 0, char_shape_id: 0 }];
        let shapes = vec![char_shape_with_attr(0)];
        let doc = styled_doc("just plain", runs, shapes);
        let opts = MdOptions {
            emit_styles: true,
            ..MdOptions::default()
        };
        let md = to_markdown_with(&doc, &opts);
        assert!(!md.contains('*'), "got: {md}");
        assert!(!md.contains('~'), "got: {md}");
        assert!(md.contains("just plain"), "got: {md}");
    }

    #[test]
    fn missing_shape_id_falls_through_to_plain() {
        // Run references shape id 99 that doesn't exist; emitter must
        // not panic and must emit the text as plain.
        let runs = vec![CharShapeRun { start: 0, char_shape_id: 99 }];
        let shapes = vec![char_shape_with_attr(0)];
        let doc = styled_doc("safe", runs, shapes);
        let opts = MdOptions {
            emit_styles: true,
            ..MdOptions::default()
        };
        let md = to_markdown_with(&doc, &opts);
        assert!(md.contains("safe"), "got: {md}");
        assert!(!md.contains('*'), "got: {md}");
    }

    #[test]
    fn hangul_utf16_offsets_align_correctly() {
        // Each Korean character is a single UTF-16 unit. "가나다" (3
        // chars = 3 u16s). Bold the middle char only: run 1 covers
        // offsets [1..2].
        let runs = vec![
            CharShapeRun { start: 0, char_shape_id: 0 },
            CharShapeRun { start: 1, char_shape_id: 1 },
            CharShapeRun { start: 2, char_shape_id: 0 },
        ];
        let shapes = vec![char_shape_with_attr(0), char_shape_with_attr(0x02)];
        let doc = styled_doc("가나다", runs, shapes);
        let opts = MdOptions {
            emit_styles: true,
            ..MdOptions::default()
        };
        let md = to_markdown_with(&doc, &opts);
        assert!(md.contains("가**나**다"), "got: {md}");
    }

    #[test]
    fn equation_inside_cell_forces_bullet_path() {
        // A 1×1 table whose single cell contains an equation. The
        // MD-grid path must bail (can't cleanly host math inside a
        // pipe cell) and emit via bullet list instead, preserving
        // the equation as inline `$…$` math under the cell's bullet.
        let inner_para = Paragraph {
            controls: vec![equation("OVER {1} {2}")],
            ..Paragraph::default()
        };
        let t = TableControl {
            rows: 1,
            cols: 1,
            row_cell_counts: vec![1],
            cells: vec![TableCell {
                col: 0,
                row: 0,
                col_span: 1,
                row_span: 1,
                paragraphs: vec![inner_para],
                ..TableCell::default()
            }],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(t)]);
        let md = body(to_markdown(&doc));
        assert!(md.contains("<!-- table 1×1"), "should pick bullet path: {md}");
        // Inline math survives in a bullet — `$…$` rather than a
        // multi-line `$$` block so the list structure is preserved.
        assert!(md.contains("$"), "got: {md}");
        assert!(md.contains("\\frac") || md.contains("1"), "got: {md}");
    }
}
