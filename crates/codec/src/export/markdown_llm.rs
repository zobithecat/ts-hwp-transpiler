//! LLM-friendly structured Markdown emitter.
//!
//! A separate surface from the human-readable `markdown` path. Where the
//! human path trades structure for prose (tables become bullet lists,
//! long titles flatten, figure placeholders shrink to `{{그림 N.}}`),
//! this path trades prose for structure: every block carries a stable
//! id so an LLM can target a specific slot, modify its text, and have
//! the mapping survive back into the original document for reinsertion.
//!
//! Output is still a valid Markdown file — nothing here requires a
//! bespoke parser — but the information layout is record-style, not
//! prose-like:
//!
//! ```text
//! SECTION[id=sec-0]
//!
//! PARAGRAPH[id=par-s0-p0]
//! TEXT: Some introductory body text.
//!
//! TABLE[id=tbl-s0-p1-c0,rows=3,cols=2]
//! CELL[id=cell-s0-p1-c0-r0c0,row=0,col=0,rowspan=1,colspan=1]
//! TEXT[par-s0-p1-c0-r0c0-p0]: 항목
//! CELL[id=cell-s0-p1-c0-r0c1,row=0,col=1,rowspan=1,colspan=1]
//! TEXT[par-s0-p1-c0-r0c1-p0]: 내용
//! ...
//! END TABLE[tbl-s0-p1-c0]
//!
//! FIGURE[id=fig-3,bin_id=3,width_mm=84,height_mm=52,caption_ref=cap-fig-3]
//! CAPTION[id=cap-fig-3,for=fig-3]
//! TEXT: 시스템 전체 아키텍처
//! ```
//!
//! **ID scheme** — purely positional, not derived from HWP5's
//! `instance_id` (empirically many 한컴 documents assign the same
//! `0x80000000` value across most paragraphs, so it's not unique enough
//! to anchor a slot). Positions don't move unless the document
//! structure changes, which is exactly the semantic we want:
//! "structure unchanged → ids unchanged". Figures and captions use
//! `bin_id` because HWP's `/BinData/BIN<id>` is always unique.
//!
//! - `sec-{si}`
//! - `par-s{si}-p{pi}` — top-level paragraph `pi` in section `si`
//! - `tbl-s{si}-p{pi}-c{ci}` — table at control index `ci` of that paragraph
//! - `cell-<tbl_id>-r{r}c{c}`
//! - Deeply nested: each level appends its own `-p{pi}-c{ci}` /
//!   `-r{r}c{c}` segment, so `tbl-s0-p5-c0-r2c3-p1-c0` is "section 0,
//!   paragraph 5, control 0 (outer table), cell (2,3), its paragraph 1,
//!   control 0 (nested table)".
//! - `fig-{bin_id}`, `cap-fig-{bin_id}` — 1:1 with `BinData` record,
//!   globally unique.

use hwp_transpiler_core::ir::{
    CharShape, ControlKind, EquationControl, IrDocument, Paragraph, PictureControl, TableCell,
    TableControl,
};
use hwp_transpiler_core::semantics::{
    CellRole, DocInfoResolver, TableDomain, VisualExtract, classify_roles,
    infer_table_domain,
};

use super::markdown::{LlmOptions, MdOptions};

/// Emit a line terminated by `\n`. Small wrapper so call sites stay
/// uncluttered; writing to a `String` is infallible so the `fmt::Result`
/// that `writeln!` would return is simply not useful here.
fn line(out: &mut String, s: &str) {
    out.push_str(s);
    out.push('\n');
}

/// Entry point. Returns the complete structured-Markdown text. The
/// caller must have set `opts.llm` to `Some(_)`; `to_markdown_with`
/// routes here automatically when the option is present.
pub fn to_llm_markdown(doc: &IrDocument, opts: &MdOptions) -> String {
    let llm = opts.llm.clone().unwrap_or_default();
    // Editable exports (`skip_section_bytes`) drop the `lineseg=`
    // carry: `vertpos` is cumulative within its list, so after any
    // edit the captured layout is stale for everything that follows.
    // Omitting it makes the writer skip `<hp:linesegarray>` and
    // Hancom re-runs line layout — correct-by-construction, and the
    // LLM context stays slimmer. Archive exports keep the carry for
    // pixel-fidelity replay of HWP5-sourced layout.
    let carry_lineseg = !opts.skip_section_bytes;
    let mut out = String::new();
    out.push_str(super::markdown::FORMAT_HEADER_LLM);
    out.push('\n');
    if let Some((id, hex)) = &llm.edit_color {
        // Editing agents read this to know which CharShape id renders
        // the "edited" colour; see docs/llm-edit-prompt.md.
        out.push_str(&format!(
            "<!-- hwp-transpiler: edit-color char_shape={id} color={hex} -->\n"
        ));
    }
    out.push('\n');
    for (si, section) in doc.sections.iter().enumerate() {
        line(&mut out, &format!("SECTION[id=sec-{si}]"));
        out.push('\n');
        for (pi, para) in section.paragraphs.iter().enumerate() {
            let par_path = format!("s{si}-p{pi}");
            emit_paragraph(doc, para, &mut out, &llm, &par_path, carry_lineseg);
        }
    }
    while out.ends_with('\n') {
        out.pop();
    }
    out.push('\n');
    if opts.asset_mode == super::markdown::AssetMode::Inline {
        let footer = super::asset_footer::render_assets_block(doc, opts);
        if !footer.is_empty() {
            out.push('\n');
            out.push_str(&footer);
            while out.ends_with('\n') {
                out.pop();
            }
            out.push('\n');
        }
    }
    out
}

/// Add a dedicated "edit colour" `CharShape` to `doc` and return its
/// id (index into `doc_info.char_shapes`). An editing agent points
/// `char_shape=<id>` at added/modified paragraphs so they render in
/// `hex` (`#RRGGBB`) after the round-trip. Set the returned id + hex on
/// `LlmOptions::edit_color` so the exporter emits the marker.
///
/// The new shape clones `char_shapes[0]` (sane font metrics) and only
/// overrides the text colour. For HWPX sources the same `<hh:charPr>` is
/// also spliced into the verbatim `Contents/header.xml` (itemCnt bumped)
/// so it survives the importer's header reparse; HWP5 sources carry it
/// through the `DOC_INFO` blob instead.
pub fn inject_edit_color(doc: &mut IrDocument, hex: &str) -> u32 {
    let id = doc.doc_info.char_shapes.len() as u32;
    let mut shape = doc.doc_info.char_shapes.first().cloned().unwrap_or_default();
    shape.color = parse_hex_color(hex);
    doc.doc_info.char_shapes.push(shape);

    if let Some(bytes) = doc.unknown_streams.get_mut("Contents/header.xml") {
        if let Ok(xml) = std::str::from_utf8(bytes) {
            if let Some(rewritten) = splice_edit_color_charpr(xml, id, hex) {
                *bytes = rewritten.into_bytes();
            }
        }
    }
    id
}

/// `#RRGGBB` → the `CharShape::color` packing (`color_to_hex` reads it
/// as `R | G<<8 | B<<16`). Falls back to red on a malformed value.
fn parse_hex_color(hex: &str) -> u32 {
    let h = hex.trim_start_matches('#');
    if h.len() == 6 {
        if let Ok(v) = u32::from_str_radix(h, 16) {
            let (r, g, b) = ((v >> 16) & 0xFF, (v >> 8) & 0xFF, v & 0xFF);
            return r | (g << 8) | (b << 16);
        }
    }
    0x0000_00FF // red
}

/// Clone the first `<hh:charPr …>…</hh:charPr>` (or self-closing form)
/// in the header, retag it as `id`, recolour its `textColor`, splice it
/// before `</hh:charProperties>`, and bump that container's `itemCnt`.
/// Returns `None` (leaving the header untouched) if the structure isn't
/// found — the DOC_INFO path then still carries the shape.
fn splice_edit_color_charpr(xml: &str, id: u32, hex: &str) -> Option<String> {
    let cp_start = xml.find("<hh:charPr")?;
    let after = &xml[cp_start..];
    // Element extent: self-closing `.../>` vs `</hh:charPr>`.
    let gt = after.find('>')?;
    let cp_end = if after.as_bytes().get(gt.wrapping_sub(1)) == Some(&b'/') {
        cp_start + gt + 1
    } else {
        cp_start + after.find("</hh:charPr>")? + "</hh:charPr>".len()
    };
    let mut cloned = xml[cp_start..cp_end].to_string();
    cloned = replace_attr(&cloned, "id", &id.to_string());
    cloned = replace_attr(&cloned, "textColor", hex);

    let close = xml.find("</hh:charProperties>")?;
    let mut out = String::with_capacity(xml.len() + cloned.len() + 16);
    out.push_str(&xml[..close]);
    out.push_str(&cloned);
    out.push_str(&xml[close..]);
    Some(bump_item_count(&out, "charProperties"))
}

/// Replace the first `name="…"` value in a single tag string. Assumes
/// the attribute is present (true for `id`/`textColor` on charPr).
fn replace_attr(tag: &str, name: &str, value: &str) -> String {
    let needle = format!("{name}=\"");
    let Some(i) = tag.find(&needle) else { return tag.to_string() };
    let vstart = i + needle.len();
    let Some(rel) = tag[vstart..].find('"') else { return tag.to_string() };
    format!("{}{}{}", &tag[..vstart], value, &tag[vstart + rel..])
}

/// Increment `<hh:{container} itemCnt="N">` by one (no-op if absent).
fn bump_item_count(xml: &str, container: &str) -> String {
    let open = format!("<hh:{container}");
    let Some(cs) = xml.find(&open) else { return xml.to_string() };
    let Some(gt) = xml[cs..].find('>') else { return xml.to_string() };
    let tag = &xml[cs..cs + gt];
    let Some(ci) = tag.find("itemCnt=\"") else { return xml.to_string() };
    let vstart = cs + ci + "itemCnt=\"".len();
    let Some(rel) = xml[vstart..].find('"') else { return xml.to_string() };
    let n: u32 = xml[vstart..vstart + rel].parse().unwrap_or(0);
    format!("{}{}{}", &xml[..vstart], n + 1, &xml[vstart + rel..])
}

/// `path` is the positional segment for this paragraph (e.g. `"s0-p5"`
/// for a top-level paragraph, `"s0-p5-c0-r2c3-p1"` for one nested in a
/// cell). Full ids prepend the kind tag: `par-<path>`.
fn emit_paragraph(
    doc: &IrDocument,
    para: &Paragraph,
    out: &mut String,
    llm: &LlmOptions,
    path: &str,
    carry_lineseg: bool,
) {
    let text = super::markdown::clean_text(&para.text);
    let has_text = !text.is_empty();
    let has_structural = para.controls.iter().any(|c| {
        matches!(
            &c.kind,
            ControlKind::Table(_) | ControlKind::Picture(_) | ControlKind::Equation(_)
        )
    });
    if has_text {
        // Stamp the heading level on the PARAGRAPH record so the LLM
        // importer can re-classify on the symmetric direction. The
        // shared `super::markdown::heading_level` lookup keys off
        // `Style::name` ("개요 N" / "Outline N"), the same convention
        // the human-Markdown exporter uses.
        let mut header = match super::markdown::heading_level(doc, para) {
            Some(n) => format!("PARAGRAPH[id=par-{path},level={n}"),
            None => format!("PARAGRAPH[id=par-{path}"),
        };
        // Stamp `para_shape` and (first-run) `char_shape` so the
        // importer can route the right HWPX `paraPrIDRef` /
        // `charPrIDRef` back. Without these, every paragraph
        // collapsed to slot 0 on round-trip and HWP5-sourced docs
        // rendered with uniform layout (same alignment / line height
        // / font size for all 1300+ paragraphs).
        header.push_str(&format!(",para_shape={}", para.header.para_shape_id));
        let first_run_shape = para
            .char_shape_runs
            .first()
            .map(|r| r.char_shape_id)
            .unwrap_or(0);
        header.push_str(&format!(",char_shape={}", first_run_shape));
        // Carry the real line layout so the HWPX writer emits the
        // exact `<hp:lineseg>` geometry instead of a single seed.
        // Needed even for single-line paragraphs: the seed's 1000-unit
        // line height rarely matches the source (e.g. 1100/1320), and
        // every under-advance accumulates into paragraph overlap down
        // the page. Carried whenever the source captured any layout.
        if carry_lineseg && !para.line_segments.is_empty() {
            header.push_str(&format!(
                ",lineseg={}",
                super::super::lineseg_codec::encode(&para.line_segments)
            ));
        }
        header.push(']');
        line(out, &header);
        line(out, &format!("TEXT: {text}"));
        out.push('\n');
    } else if !has_structural {
        // Fully empty paragraph — skip. It carries only layout metadata
        // that doesn't help an LLM locate content.
        return;
    }

    for (ci, c) in para.controls.iter().enumerate() {
        let ctrl_path = format!("{path}-c{ci}");
        match &c.kind {
            ControlKind::Table(t) => {
                emit_table(doc, t, out, llm, &ctrl_path, &para.text, carry_lineseg);
            }
            ControlKind::Picture(p) => {
                emit_figure(p, c.caption_text.as_deref(), out);
            }
            ControlKind::Equation(eq) => {
                emit_equation(eq, out, &ctrl_path);
            }
            _ => {}
        }
    }
}

fn emit_table(
    doc: &IrDocument,
    t: &TableControl,
    out: &mut String,
    llm: &LlmOptions,
    path: &str,
    owner_para_text: &str,
    carry_lineseg: bool,
) {
    let tbl_id = format!("tbl-{path}");
    let mut tbl_header =
        format!("TABLE[id={tbl_id},rows={},cols={}", t.rows, t.cols);
    if llm.domain_hints {
        let domain = infer_table_domain(t, owner_para_text);
        if domain != TableDomain::Unknown {
            tbl_header.push_str(&format!(",kind={}", domain.as_str()));
        }
    }
    // `border_fill` carries the table-level BorderFill slot id so the
    // importer can route it back into `<hp:tbl borderFillIDRef=N>`.
    tbl_header.push_str(&format!(",border_fill={}", t.border_fill_id));
    tbl_header.push(']');
    line(out, &tbl_header);

    // Run the visual classifier whenever either role or editable is
    // requested — editable inference depends on role, so even when the
    // user only asks for editable output we compute roles internally.
    // classify_roles must see every cell at once to decide "is there
    // any label tone in this table", so it's a per-table call.
    let need_roles = llm.emit_roles || llm.emit_editable;
    let roles = if need_roles {
        compute_roles(doc, t)
    } else {
        Vec::new()
    };

    for (ci, cell) in t.cells.iter().enumerate() {
        let cell_path = format!("{path}-r{}c{}", cell.row, cell.col);
        let role = roles.get(ci).copied();
        emit_cell(doc, cell, out, llm, &cell_path, role, carry_lineseg);
    }
    line(out, &format!("END TABLE[{tbl_id}]"));
    out.push('\n');
}

/// Build a fingerprint for each cell in `t` against `doc`'s DocInfo
/// resolver, then let `classify_roles` map them to the four semantic
/// roles. Returns a vector parallel to `t.cells`. When the border-fill
/// resolver can't find a colour (id == 0, missing entry, non-color
/// fill), the fingerprint's `bg` falls to `BgTone::None` and the
/// classifier collapses to its position-only fallback — which is
/// correct for tables without colour-coded labels.
pub(super) fn compute_roles(doc: &IrDocument, t: &TableControl) -> Vec<CellRole> {
    let resolver = DocInfoResolver::new(&doc.doc_info);
    let fingerprints: Vec<_> = t
        .cells
        .iter()
        .map(|c| c.fingerprint(&resolver, c.row == 0, c.col == 0))
        .collect();
    classify_roles(&fingerprints)
}

pub(super) fn role_name(role: Option<CellRole>) -> &'static str {
    match role {
        Some(CellRole::Header) => "header",
        Some(CellRole::Label) => "label",
        Some(CellRole::Content) => "value",
        Some(CellRole::Spacer) => "spacer",
        None => "unknown",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Editable {
    Yes,
    No,
    Unknown,
}

pub(super) fn editable_name(e: Editable) -> &'static str {
    match e {
        Editable::Yes => "true",
        Editable::No => "false",
        Editable::Unknown => "unknown",
    }
}

/// Conservative editable inference. Errs toward `false` — if we can't
/// be sure the cell is a free-form text slot, we shouldn't suggest an
/// LLM rewrite it. Rules (all must hold for `true`):
///
///   1. Role is `Content` (value). Header / Label / Spacer describe
///      form structure and must never change.
///   2. At most one paragraph in the cell. Multi-paragraph cells hold
///      structured content (bullet lists, subheadings) where blind
///      rewrite risks losing the layering.
///   3. No inline controls — any nested table, picture, or equation
///      means the cell is a container, not a plain text slot.
///   4. Text is not purely numeric/punctuation (`80%+` such chars).
///      Numeric-only cells are typically calculated totals or fixed
///      dates that shouldn't be re-authored.
///
/// Empty value cells *are* editable (fill-in slots). `None` role (when
/// classifier didn't run) yields `Unknown`.
pub(super) fn infer_editable(cell: &TableCell, role: Option<CellRole>) -> Editable {
    match role {
        None => return Editable::Unknown,
        Some(CellRole::Header) | Some(CellRole::Label) | Some(CellRole::Spacer) => {
            return Editable::No;
        }
        Some(CellRole::Content) => {}
    }

    if cell.paragraphs.len() > 1 {
        return Editable::No;
    }
    let has_controls = cell
        .paragraphs
        .iter()
        .any(|p| !p.controls.is_empty());
    if has_controls {
        return Editable::No;
    }

    let text: String = cell
        .paragraphs
        .first()
        .map(|p| super::markdown::clean_text(&p.text))
        .unwrap_or_default();
    if text.is_empty() {
        // Empty value cell — a fill-in slot.
        return Editable::Yes;
    }
    if is_mostly_numeric(&text) {
        return Editable::No;
    }
    Editable::Yes
}

/// Heuristic: ≥90% of non-whitespace characters fall into the numeric
/// / punctuation / currency set → treat as a calculated/formatted
/// value that shouldn't be rewritten. Tuned against TRL fixture
/// patterns like `"134,500"`, `"100%"`, `"2026-04-22"`.
fn is_mostly_numeric(s: &str) -> bool {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return false;
    }
    let total = trimmed.chars().count();
    let numeric_like = trimmed
        .chars()
        .filter(|c| {
            c.is_ascii_digit()
                || matches!(
                    *c,
                    '.' | ',' | '-' | '+' | '%' | '/' | '(' | ')' | ' ' | ':'
                )
        })
        .count();
    numeric_like * 10 >= total * 9
}

fn emit_cell(
    doc: &IrDocument,
    cell: &TableCell,
    out: &mut String,
    llm: &LlmOptions,
    path: &str,
    role: Option<CellRole>,
    carry_lineseg: bool,
) {
    let cell_id = format!("cell-{path}");
    let mut header = format!(
        "CELL[id={cell_id},row={},col={},rowspan={},colspan={}",
        cell.row, cell.col, cell.row_span, cell.col_span
    );
    if llm.emit_roles {
        header.push_str(&format!(",role={}", role_name(role)));
    }
    if llm.emit_editable {
        let ed = infer_editable(cell, role);
        header.push_str(&format!(",editable={}", editable_name(ed)));
    }
    // `border_fill` attaches the IR's per-cell `BorderFill` slot id
    // so the importer can route the right border style back into
    // `<hp:tc borderFillIDRef=N>`. HWP5-sourced docs use 30+ slots
    // (one per visual style); without this every cell collapsed to
    // the default slot 1 on round-trip — table borders disappeared
    // because 1 was the skeleton's plain SOLID 0.12mm and the source
    // had richer styles per cell.
    header.push_str(&format!(",border_fill={}", cell.border_fill_id));
    // Carry the cell's real geometry (HWPUNIT). Without it the
    // importer falls back to evenly-distributing the page width across
    // columns, so a narrow label column (e.g. 9648) balloons to half
    // the table and its DISTRIBUTE-aligned text smears edge to edge.
    // Width (and text_width) only — they fix the column proportions
    // (narrow label vs wide value). Height is deliberately NOT carried:
    // Hancom auto-grows each row to fit its content, and forcing the
    // source's laid-out height makes the table overflow and spill onto
    // the next page (the row heights double-count against reflowed
    // content). Let the viewer size rows itself.
    if cell.width_hwpu > 0 {
        header.push_str(&format!(",width={}", cell.width_hwpu));
    }
    if cell.text_width_hwpu > 0 {
        header.push_str(&format!(",text_width={}", cell.text_width_hwpu));
    }
    header.push(']');
    line(out, &header);

    for (pi, p) in cell.paragraphs.iter().enumerate() {
        let text = super::markdown::clean_text(&p.text);
        let inner_par_path = format!("{path}-p{pi}");
        if !text.is_empty() {
            // Stamp `para_shape` / `char_shape` on cell text records
            // too. Cell paragraphs aren't routed through the top-
            // level PARAGRAPH path so the only place to carry the
            // slot ids is the TEXT bracket. HWP5-sourced cells often
            // use varied shapes (label vs value, header row, …) —
            // without these attrs every cell collapsed to slot 0 on
            // round-trip.
            let cs = p
                .char_shape_runs
                .first()
                .map(|r| r.char_shape_id)
                .unwrap_or(0);
            let lineseg_attr = if carry_lineseg && !p.line_segments.is_empty() {
                format!(
                    ",lineseg={}",
                    super::super::lineseg_codec::encode(&p.line_segments)
                )
            } else {
                String::new()
            };
            line(out, &format!(
                "TEXT[par-{inner_par_path},para_shape={ps},char_shape={cs}{lineseg_attr}]: {text}",
                ps = p.header.para_shape_id,
            ));
        }
        for (ci, ctrl) in p.controls.iter().enumerate() {
            let inner_ctrl_path = format!("{inner_par_path}-c{ci}");
            match &ctrl.kind {
                ControlKind::Table(nested) => {
                    emit_table(doc, nested, out, llm, &inner_ctrl_path, &p.text, carry_lineseg);
                }
                ControlKind::Picture(pic) => {
                    emit_figure(pic, ctrl.caption_text.as_deref(), out);
                }
                ControlKind::Equation(eq) => {
                    emit_equation(eq, out, &inner_ctrl_path);
                }
                _ => {}
            }
        }
    }
}

/// Structured EQUATION record: header line with metadata + a SCRIPT
/// line carrying the raw equation source. Follows the same shape as
/// FIGURE / TABLE / CELL so an LLM parser can ingest the three with
/// one grammar. Multi-line scripts emit multiple SCRIPT lines — one
/// per source line — so the reader can reconstruct layout without an
/// escaping convention.
///
/// Font and size_hwpu are surfaced as attributes when set; HWP
/// equations often carry a font for Korean glyphs inside the math
/// expression, which an LLM rewriting a related cell should be aware
/// of. `size_hwpu` is in HWP units (1 pt = 100 HWPUNIT).
fn emit_equation(eq: &EquationControl, out: &mut String, path: &str) {
    let eqn_id = format!("eqn-{path}");
    let mut header = format!("EQUATION[id={eqn_id}");
    if let Some(font) = &eq.font {
        if !font.is_empty() {
            header.push_str(&format!(",font={font}"));
        }
    }
    if eq.size_hwpu > 0 {
        header.push_str(&format!(",size_hwpu={}", eq.size_hwpu));
    }
    header.push(']');
    line(out, &header);
    let script = eq.script.trim_end();
    if script.is_empty() {
        line(out, "SCRIPT:");
    } else {
        for s in script.lines() {
            line(out, &format!("SCRIPT: {s}"));
        }
        // Emit a best-effort LaTeX rendering alongside the script so
        // downstream consumers that want math display (KaTeX /
        // MathJax) can use it directly, while the SCRIPT line still
        // anchors the original HWP source for exact edits.
        let latex =
            hwp_transpiler_core::formula::to_latex(script);
        if !latex.is_empty() {
            line(out, &format!("LATEX: {latex}"));
        }
    }
    out.push('\n');
}

fn emit_figure(pic: &PictureControl, caption_text: Option<&str>, out: &mut String) {
    let fig_id = format!("fig-{}", pic.bin_id);
    let w_mm = hwpunit_to_mm(pic.width_hwpu);
    let h_mm = hwpunit_to_mm(pic.height_hwpu);
    let mut header = format!(
        "FIGURE[id={fig_id},bin_id={},width_mm={w_mm},height_mm={h_mm}",
        pic.bin_id
    );
    if caption_text.is_some() {
        header.push_str(&format!(",caption_ref=cap-{fig_id}"));
    }
    header.push(']');
    line(out, &header);
    if let Some(cap) = caption_text {
        let cleaned = super::markdown::clean_text(cap);
        let stripped =
            super::markdown::strip_caption_label_prefix(&cleaned).trim();
        line(out, &format!("CAPTION[id=cap-{fig_id},for={fig_id}]"));
        line(out, &format!("TEXT: {stripped}"));
    }
    out.push('\n');
}

fn hwpunit_to_mm(hwpu: u32) -> u32 {
    ((hwpu as f64) * 25.4 / 7200.0).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use hwp_transpiler_core::ir::{
        Control, IrDocument, Paragraph, ParagraphHeader, Section, TableCell,
        TableControl,
    };

    fn para_text(text: &str) -> Paragraph {
        Paragraph {
            header: ParagraphHeader::default(),
            text: text.into(),
            ..Paragraph::default()
        }
    }

    fn opts_llm() -> MdOptions {
        MdOptions {
            llm: Some(LlmOptions::default()),
            ..MdOptions::default()
        }
    }

    #[test]
    fn minimal_section_with_one_paragraph() {
        let mut doc = IrDocument::default();
        doc.sections.push(Section {
            paragraphs: vec![para_text("hello")],
            ..Section::default()
        });
        let md = to_llm_markdown(&doc, &opts_llm());
        assert!(md.contains("SECTION[id=sec-0]"), "got: {md}");
        assert!(md.contains("PARAGRAPH[id=par-s0-p0"), "got: {md}");
        assert!(md.contains("TEXT: hello"), "got: {md}");
    }

    #[test]
    fn table_cell_id_is_positional_path() {
        let t = TableControl {
            rows: 1, cols: 2, row_cell_counts: vec![2],
            cells: vec![
                TableCell {
                    col: 0, row: 0, col_span: 1, row_span: 1,
                    paragraphs: vec![para_text("A")],
                    ..TableCell::default()
                },
                TableCell {
                    col: 1, row: 0, col_span: 1, row_span: 1,
                    paragraphs: vec![para_text("B")],
                    ..TableCell::default()
                },
            ],
            ..TableControl::default()
        };
        let mut doc = IrDocument::default();
        // Top-level paragraph at section 0, index 3, hosting one table control.
        let mut paras: Vec<Paragraph> = (0..3).map(|_| Paragraph::default()).collect();
        paras.push(Paragraph {
            controls: vec![Control { kind: ControlKind::Table(t), ..Default::default() }],
            ..Paragraph::default()
        });
        doc.sections.push(Section { paragraphs: paras, ..Section::default() });
        let md = to_llm_markdown(&doc, &opts_llm());
        assert!(md.contains("TABLE[id=tbl-s0-p3-c0,rows=1,cols=2"), "got: {md}");
        assert!(
            md.contains("CELL[id=cell-s0-p3-c0-r0c0,row=0,col=0,rowspan=1,colspan=1"),
            "got: {md}"
        );
        assert!(
            md.contains("CELL[id=cell-s0-p3-c0-r0c1,row=0,col=1,rowspan=1,colspan=1"),
            "got: {md}"
        );
        assert!(md.contains("TEXT[par-s0-p3-c0-r0c0-p0,"), "got: {md}");
        assert!(md.contains("]: A"), "got: {md}");
        assert!(md.contains("TEXT[par-s0-p3-c0-r0c1-p0,"), "got: {md}");
        assert!(md.contains("]: B"), "got: {md}");
        assert!(md.contains("END TABLE[tbl-s0-p3-c0]"), "got: {md}");
    }

    #[test]
    fn nested_table_id_accumulates_path() {
        let inner = TableControl {
            rows: 1, cols: 1, row_cell_counts: vec![1],
            cells: vec![TableCell {
                col: 0, row: 0, col_span: 1, row_span: 1,
                paragraphs: vec![para_text("inner")],
                ..TableCell::default()
            }],
            ..TableControl::default()
        };
        let outer = TableControl {
            rows: 1, cols: 1, row_cell_counts: vec![1],
            cells: vec![TableCell {
                col: 0, row: 0, col_span: 1, row_span: 1,
                paragraphs: vec![Paragraph {
                    text: "wrapper text".into(),
                    controls: vec![Control { kind: ControlKind::Table(inner), ..Default::default() }],
                    ..Paragraph::default()
                }],
                ..TableCell::default()
            }],
            ..TableControl::default()
        };
        let mut doc = IrDocument::default();
        doc.sections.push(Section {
            paragraphs: vec![Paragraph {
                controls: vec![Control { kind: ControlKind::Table(outer), ..Default::default() }],
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        let md = to_llm_markdown(&doc, &opts_llm());
        assert!(md.contains("TABLE[id=tbl-s0-p0-c0,"), "got: {md}");
        assert!(
            md.contains("TABLE[id=tbl-s0-p0-c0-r0c0-p0-c0,"),
            "nested table id missing lineage: {md}"
        );
    }

    #[test]
    fn figure_with_caption_emits_linked_pair() {
        use hwp_transpiler_core::ir::PictureControl;
        let mut doc = IrDocument::default();
        doc.sections.push(Section {
            paragraphs: vec![Paragraph {
                controls: vec![Control {
                    kind: ControlKind::Picture(PictureControl {
                        bin_id: 3,
                        width_hwpu: 7200,
                        height_hwpu: 3600,
                    }),
                    caption_text: Some("그림 \u{FFFC}. 시스템 도식".into()),
                }],
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        let md = to_llm_markdown(&doc, &opts_llm());
        assert!(
            md.contains("FIGURE[id=fig-3,bin_id=3,width_mm=25,height_mm=13,caption_ref=cap-fig-3]"),
            "got: {md}"
        );
        assert!(md.contains("CAPTION[id=cap-fig-3,for=fig-3]"), "got: {md}");
        assert!(md.contains("TEXT: 시스템 도식"), "got: {md}");
        assert!(!md.contains('\u{FFFC}'), "no raw FFFC leak");
    }

    #[test]
    fn figure_without_caption_omits_caption_ref() {
        use hwp_transpiler_core::ir::PictureControl;
        let mut doc = IrDocument::default();
        doc.sections.push(Section {
            paragraphs: vec![Paragraph {
                controls: vec![Control {
                    kind: ControlKind::Picture(PictureControl {
                        bin_id: 1, width_hwpu: 0, height_hwpu: 0,
                    }),
                    caption_text: None,
                }],
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        let md = to_llm_markdown(&doc, &opts_llm());
        assert!(md.contains("FIGURE[id=fig-1,bin_id=1"), "got: {md}");
        assert!(!md.contains("caption_ref="), "got: {md}");
        assert!(!md.contains("CAPTION["), "got: {md}");
    }

    #[test]
    fn uncoloured_single_cell_is_editable_value() {
        // No DocInfo border-fills → resolver returns None → BgTone::None.
        // Cell at (0,0) is first_row AND first_col — classify_roles falls
        // to the Content branch (value). Single plain-text paragraph,
        // not numeric → editable.
        let t = TableControl {
            rows: 1, cols: 1, row_cell_counts: vec![1],
            cells: vec![TableCell {
                col: 0, row: 0, col_span: 1, row_span: 1,
                paragraphs: vec![para_text("x")],
                ..TableCell::default()
            }],
            ..TableControl::default()
        };
        let mut doc = IrDocument::default();
        doc.sections.push(Section {
            paragraphs: vec![Paragraph {
                controls: vec![Control { kind: ControlKind::Table(t), ..Default::default() }],
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        let opts = MdOptions {
            llm: Some(LlmOptions { emit_roles: true, emit_editable: true, ..LlmOptions::default() }),
            ..MdOptions::default()
        };
        let md = to_llm_markdown(&doc, &opts);
        assert!(md.contains("role=value"), "got: {md}");
        assert!(md.contains("editable=true"), "got: {md}");
    }

    #[test]
    fn empty_value_cell_is_editable() {
        let t = TableControl {
            rows: 1, cols: 1, row_cell_counts: vec![1],
            cells: vec![TableCell {
                col: 0, row: 0, col_span: 1, row_span: 1,
                paragraphs: vec![para_text("")],
                ..TableCell::default()
            }],
            ..TableControl::default()
        };
        let mut doc = IrDocument::default();
        doc.sections.push(Section {
            paragraphs: vec![Paragraph {
                controls: vec![Control { kind: ControlKind::Table(t), ..Default::default() }],
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        let opts = MdOptions {
            llm: Some(LlmOptions { emit_roles: true, emit_editable: true, ..LlmOptions::default() }),
            ..MdOptions::default()
        };
        let md = to_llm_markdown(&doc, &opts);
        assert!(md.contains("editable=true"), "empty slot should be editable; got: {md}");
    }

    #[test]
    fn numeric_only_cell_is_not_editable() {
        let t = TableControl {
            rows: 1, cols: 1, row_cell_counts: vec![1],
            cells: vec![TableCell {
                col: 0, row: 0, col_span: 1, row_span: 1,
                paragraphs: vec![para_text("134,500")],
                ..TableCell::default()
            }],
            ..TableControl::default()
        };
        let mut doc = IrDocument::default();
        doc.sections.push(Section {
            paragraphs: vec![Paragraph {
                controls: vec![Control { kind: ControlKind::Table(t), ..Default::default() }],
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        let opts = MdOptions {
            llm: Some(LlmOptions { emit_roles: true, emit_editable: true, ..LlmOptions::default() }),
            ..MdOptions::default()
        };
        let md = to_llm_markdown(&doc, &opts);
        assert!(md.contains("editable=false"), "numeric-only must be non-editable; got: {md}");
    }

    #[test]
    fn label_cell_is_never_editable() {
        use hwp_transpiler_core::ir::{BorderFill, Fill};
        let mut doc = IrDocument::default();
        doc.doc_info.border_fills.push(BorderFill {
            fill: Fill {
                kind: Fill::KIND_COLOR,
                body: vec![0xFF, 0xFF, 0x99, 0x00, 0, 0, 0, 0],
            },
            ..BorderFill::default()
        });
        let t = TableControl {
            rows: 1, cols: 1, row_cell_counts: vec![1],
            cells: vec![TableCell {
                col: 0, row: 0, col_span: 1, row_span: 1,
                border_fill_id: 1,
                paragraphs: vec![para_text("항목")],
                ..TableCell::default()
            }],
            ..TableControl::default()
        };
        doc.sections.push(Section {
            paragraphs: vec![Paragraph {
                controls: vec![Control { kind: ControlKind::Table(t), ..Default::default() }],
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        let opts = MdOptions {
            llm: Some(LlmOptions { emit_roles: true, emit_editable: true, ..LlmOptions::default() }),
            ..MdOptions::default()
        };
        let md = to_llm_markdown(&doc, &opts);
        assert!(md.contains("role=label"));
        assert!(
            md.contains("editable=false"),
            "label cells must be non-editable; got: {md}"
        );
    }

    #[test]
    fn cell_with_nested_control_is_not_editable() {
        use hwp_transpiler_core::ir::PictureControl;
        // Cell holds a picture control → structural, not a text slot.
        let t = TableControl {
            rows: 1, cols: 1, row_cell_counts: vec![1],
            cells: vec![TableCell {
                col: 0, row: 0, col_span: 1, row_span: 1,
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
                ..TableCell::default()
            }],
            ..TableControl::default()
        };
        let mut doc = IrDocument::default();
        doc.sections.push(Section {
            paragraphs: vec![Paragraph {
                controls: vec![Control { kind: ControlKind::Table(t), ..Default::default() }],
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        let opts = MdOptions {
            llm: Some(LlmOptions { emit_roles: true, emit_editable: true, ..LlmOptions::default() }),
            ..MdOptions::default()
        };
        let md = to_llm_markdown(&doc, &opts);
        assert!(
            md.contains("editable=false"),
            "cell with picture must be non-editable; got: {md}"
        );
    }

    #[test]
    fn multi_paragraph_cell_is_not_editable() {
        let t = TableControl {
            rows: 1, cols: 1, row_cell_counts: vec![1],
            cells: vec![TableCell {
                col: 0, row: 0, col_span: 1, row_span: 1,
                paragraphs: vec![para_text("첫 문단"), para_text("둘째 문단")],
                ..TableCell::default()
            }],
            ..TableControl::default()
        };
        let mut doc = IrDocument::default();
        doc.sections.push(Section {
            paragraphs: vec![Paragraph {
                controls: vec![Control { kind: ControlKind::Table(t), ..Default::default() }],
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        let opts = MdOptions {
            llm: Some(LlmOptions { emit_roles: true, emit_editable: true, ..LlmOptions::default() }),
            ..MdOptions::default()
        };
        let md = to_llm_markdown(&doc, &opts);
        assert!(md.contains("editable=false"), "got: {md}");
    }

    #[test]
    fn editable_flag_alone_still_computes_roles_internally() {
        // emit_editable=true, emit_roles=false. User should see editable
        // but not role; internal role computation still happens so the
        // editable verdict is meaningful.
        let t = TableControl {
            rows: 1, cols: 1, row_cell_counts: vec![1],
            cells: vec![TableCell {
                col: 0, row: 0, col_span: 1, row_span: 1,
                paragraphs: vec![para_text("일반 텍스트")],
                ..TableCell::default()
            }],
            ..TableControl::default()
        };
        let mut doc = IrDocument::default();
        doc.sections.push(Section {
            paragraphs: vec![Paragraph {
                controls: vec![Control { kind: ControlKind::Table(t), ..Default::default() }],
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        let opts = MdOptions {
            llm: Some(LlmOptions {
                emit_roles: false,
                emit_editable: true,
                ..LlmOptions::default()
            }),
            ..MdOptions::default()
        };
        let md = to_llm_markdown(&doc, &opts);
        assert!(md.contains("editable=true"), "got: {md}");
        assert!(!md.contains("role="), "role should NOT be emitted: {md}");
    }

    #[test]
    fn domain_hint_attaches_to_table_when_flag_on() {
        // Budget-like table (keyword hits: 정부지원 + 기관 현금 + 합계).
        let t = TableControl {
            rows: 1, cols: 4, row_cell_counts: vec![4],
            cells: vec![
                TableCell {
                    col: 0, row: 0, col_span: 1, row_span: 1,
                    paragraphs: vec![para_text("구분")],
                    ..TableCell::default()
                },
                TableCell {
                    col: 1, row: 0, col_span: 1, row_span: 1,
                    paragraphs: vec![para_text("정부지원")],
                    ..TableCell::default()
                },
                TableCell {
                    col: 2, row: 0, col_span: 1, row_span: 1,
                    paragraphs: vec![para_text("기관 현금")],
                    ..TableCell::default()
                },
                TableCell {
                    col: 3, row: 0, col_span: 1, row_span: 1,
                    paragraphs: vec![para_text("합계")],
                    ..TableCell::default()
                },
            ],
            ..TableControl::default()
        };
        let mut doc = IrDocument::default();
        doc.sections.push(Section {
            paragraphs: vec![Paragraph {
                controls: vec![Control { kind: ControlKind::Table(t), ..Default::default() }],
                ..Paragraph::default()
            }],
            ..Section::default()
        });

        // Flag off → no kind attribute.
        let md_off = to_llm_markdown(&doc, &opts_llm());
        assert!(!md_off.contains("kind="));

        // Flag on → kind=budget on the TABLE marker.
        let md_on = to_llm_markdown(
            &doc,
            &MdOptions {
                llm: Some(LlmOptions {
                    domain_hints: true,
                    ..LlmOptions::default()
                }),
                ..MdOptions::default()
            },
        );
        assert!(
            md_on.contains("TABLE[id=tbl-s0-p0-c0,rows=1,cols=4,kind=budget"),
            "got: {md_on}"
        );
    }

    #[test]
    fn domain_hint_unknown_is_elided() {
        // Cells have no domain keywords → Unknown → no `kind=` emitted.
        let t = TableControl {
            rows: 1, cols: 2, row_cell_counts: vec![2],
            cells: vec![
                TableCell {
                    col: 0, row: 0, col_span: 1, row_span: 1,
                    paragraphs: vec![para_text("A")],
                    ..TableCell::default()
                },
                TableCell {
                    col: 1, row: 0, col_span: 1, row_span: 1,
                    paragraphs: vec![para_text("B")],
                    ..TableCell::default()
                },
            ],
            ..TableControl::default()
        };
        let mut doc = IrDocument::default();
        doc.sections.push(Section {
            paragraphs: vec![Paragraph {
                controls: vec![Control { kind: ControlKind::Table(t), ..Default::default() }],
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        let md = to_llm_markdown(
            &doc,
            &MdOptions {
                llm: Some(LlmOptions {
                    domain_hints: true,
                    ..LlmOptions::default()
                }),
                ..MdOptions::default()
            },
        );
        assert!(!md.contains("kind="), "unknown must elide: {md}");
    }

    #[test]
    fn is_mostly_numeric_covers_common_patterns() {
        // Calculated / formatted values — numeric-only set.
        assert!(is_mostly_numeric("134,500"));
        assert!(is_mostly_numeric("100%"));
        assert!(is_mostly_numeric("2026-04-22"));
        assert!(is_mostly_numeric("3.14"));
        assert!(is_mostly_numeric("1/2"));

        // Korean-heavy — editable candidates.
        assert!(!is_mostly_numeric("연구개발계획서"));
        assert!(!is_mostly_numeric("1단계"));
        assert!(!is_mostly_numeric("2024 연구"));

        // Edge: empty.
        assert!(!is_mostly_numeric(""));
    }

    #[test]
    fn yellow_fill_cell_gets_label_role() {
        use hwp_transpiler_core::ir::{BorderFill, Fill};
        // DocInfo has one yellow border-fill at id=1. Cell references it.
        let mut doc = IrDocument::default();
        doc.doc_info.border_fills.push(BorderFill {
            fill: Fill {
                kind: Fill::KIND_COLOR,
                // R=0xFF, G=0xFF, B=0x99, A=0xFF → pale yellow → Hue::Yellow
                body: vec![0xFF, 0xFF, 0x99, 0xFF, 0, 0, 0, 0],
            },
            ..BorderFill::default()
        });
        let t = TableControl {
            rows: 1, cols: 2, row_cell_counts: vec![2],
            cells: vec![
                TableCell {
                    col: 0, row: 0, col_span: 1, row_span: 1,
                    border_fill_id: 1,
                    paragraphs: vec![para_text("항목")],
                    ..TableCell::default()
                },
                TableCell {
                    col: 1, row: 0, col_span: 1, row_span: 1,
                    border_fill_id: 0,
                    paragraphs: vec![para_text("내용")],
                    ..TableCell::default()
                },
            ],
            ..TableControl::default()
        };
        doc.sections.push(Section {
            paragraphs: vec![Paragraph {
                controls: vec![Control { kind: ControlKind::Table(t), ..Default::default() }],
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        let opts = MdOptions {
            llm: Some(LlmOptions { emit_roles: true, ..LlmOptions::default() }),
            ..MdOptions::default()
        };
        let md = to_llm_markdown(&doc, &opts);
        // First cell (yellow) → label. Second cell → header (first_row
        // non-first_col path); content has to wait for a row-2 example.
        assert!(
            md.contains("CELL[id=cell-s0-p0-c0-r0c0,row=0,col=0,rowspan=1,colspan=1,role=label,"),
            "expected label on yellow cell; got: {md}"
        );
    }

    #[test]
    fn dark_fill_cell_gets_header_role() {
        use hwp_transpiler_core::ir::{BorderFill, Fill};
        let mut doc = IrDocument::default();
        doc.doc_info.border_fills.push(BorderFill {
            fill: Fill {
                kind: Fill::KIND_COLOR,
                // Near-gray with luminance ~30 → BgTone::Dark
                body: vec![0x1E, 0x1E, 0x1E, 0xFF, 0, 0, 0, 0],
            },
            ..BorderFill::default()
        });
        let t = TableControl {
            rows: 1, cols: 1, row_cell_counts: vec![1],
            cells: vec![TableCell {
                col: 0, row: 0, col_span: 1, row_span: 1,
                border_fill_id: 1,
                paragraphs: vec![para_text("구분")],
                ..TableCell::default()
            }],
            ..TableControl::default()
        };
        doc.sections.push(Section {
            paragraphs: vec![Paragraph {
                controls: vec![Control { kind: ControlKind::Table(t), ..Default::default() }],
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        let opts = MdOptions {
            llm: Some(LlmOptions { emit_roles: true, ..LlmOptions::default() }),
            ..MdOptions::default()
        };
        let md = to_llm_markdown(&doc, &opts);
        assert!(
            md.contains("role=header"),
            "dark fill must classify as header; got: {md}"
        );
    }

    #[test]
    fn output_is_deterministic_across_runs() {
        // Same IR → same bytes. Guards against any map/hashing creeping
        // into the emitter.
        let mut doc = IrDocument::default();
        doc.sections.push(Section {
            paragraphs: vec![para_text("a"), para_text("b")],
            ..Section::default()
        });
        let md1 = to_llm_markdown(&doc, &opts_llm());
        let md2 = to_llm_markdown(&doc, &opts_llm());
        assert_eq!(md1, md2);
    }

    #[test]
    fn id_stability_unaffected_by_non_unique_instance_id() {
        // Regression guard: empirical TRL fixtures set
        // ParagraphHeader.instance_id = 0x80000000 for most paragraphs.
        // If the emitter ever tries to derive ids from instance_id
        // again, this test catches the collision.
        let mut doc = IrDocument::default();
        let same = ParagraphHeader {
            instance_id: 0x8000_0000,
            ..Default::default()
        };
        doc.sections.push(Section {
            paragraphs: vec![
                Paragraph { header: same.clone(), text: "a".into(), ..Default::default() },
                Paragraph { header: same.clone(), text: "b".into(), ..Default::default() },
                Paragraph { header: same,         text: "c".into(), ..Default::default() },
            ],
            ..Section::default()
        });
        let md = to_llm_markdown(&doc, &opts_llm());
        assert!(md.contains("PARAGRAPH[id=par-s0-p0"));
        assert!(md.contains("PARAGRAPH[id=par-s0-p1"));
        assert!(md.contains("PARAGRAPH[id=par-s0-p2"));
    }

    fn equation_para(script: &str, font: Option<&str>, size_hwpu: i32) -> Paragraph {
        use hwp_transpiler_core::ir::EquationControl;
        Paragraph {
            controls: vec![Control {
                kind: ControlKind::Equation(EquationControl {
                    script: script.into(),
                    font: font.map(|f| f.into()),
                    size_hwpu,
                }),
                ..Default::default()
            }],
            ..Paragraph::default()
        }
    }

    #[test]
    fn equation_emits_structured_record() {
        let mut doc = IrDocument::default();
        doc.sections.push(Section {
            paragraphs: vec![equation_para("x^2 + y^2", Some("명조"), 1200)],
            ..Section::default()
        });
        let md = to_llm_markdown(&doc, &opts_llm());
        assert!(
            md.contains("EQUATION[id=eqn-s0-p0-c0,font=명조,size_hwpu=1200]"),
            "got: {md}"
        );
        assert!(md.contains("SCRIPT: x^2 + y^2"), "got: {md}");
    }

    #[test]
    fn equation_without_font_or_size_omits_attrs() {
        let mut doc = IrDocument::default();
        doc.sections.push(Section {
            paragraphs: vec![equation_para("a = b", None, 0)],
            ..Section::default()
        });
        let md = to_llm_markdown(&doc, &opts_llm());
        assert!(md.contains("EQUATION[id=eqn-s0-p0-c0]"), "got: {md}");
        assert!(!md.contains("font="), "got: {md}");
        assert!(!md.contains("size_hwpu="), "got: {md}");
    }

    #[test]
    fn equation_multiline_script_emits_one_line_per_source_line() {
        let mut doc = IrDocument::default();
        doc.sections.push(Section {
            paragraphs: vec![equation_para("over{a}{b}\n= c", None, 0)],
            ..Section::default()
        });
        let md = to_llm_markdown(&doc, &opts_llm());
        assert!(md.contains("SCRIPT: over{a}{b}"), "got: {md}");
        assert!(md.contains("SCRIPT: = c"), "got: {md}");
    }

    #[test]
    fn equation_inside_table_cell_uses_nested_path() {
        // Equation inside a 1×1 cell should get the nested id
        // `eqn-s0-p0-c0-r0c0-p0-c0` so an LLM can locate it
        // unambiguously.
        use hwp_transpiler_core::ir::EquationControl;
        let cell_para = Paragraph {
            controls: vec![Control {
                kind: ControlKind::Equation(EquationControl {
                    script: "over{1}{2}".into(),
                    font: None,
                    size_hwpu: 0,
                }),
                ..Default::default()
            }],
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
                paragraphs: vec![cell_para],
                ..TableCell::default()
            }],
            ..TableControl::default()
        };
        let mut doc = IrDocument::default();
        doc.sections.push(Section {
            paragraphs: vec![Paragraph {
                controls: vec![Control {
                    kind: ControlKind::Table(t),
                    ..Default::default()
                }],
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        let md = to_llm_markdown(&doc, &opts_llm());
        assert!(
            md.contains("EQUATION[id=eqn-s0-p0-c0-r0c0-p0-c0]"),
            "got: {md}"
        );
        assert!(md.contains("SCRIPT: over{1}{2}"), "got: {md}");
    }
}
