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
    ControlKind, IrDocument, Paragraph, PictureControl, TableCell, TableControl,
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
    let mut out = String::new();
    for (si, section) in doc.sections.iter().enumerate() {
        line(&mut out, &format!("SECTION[id=sec-{si}]"));
        out.push('\n');
        for (pi, para) in section.paragraphs.iter().enumerate() {
            let par_path = format!("s{si}-p{pi}");
            emit_paragraph(doc, para, &mut out, &llm, &par_path);
        }
    }
    while out.ends_with('\n') {
        out.pop();
    }
    out.push('\n');
    out
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
) {
    let text = super::markdown::clean_text(&para.text);
    let has_text = !text.is_empty();
    let has_structural = para.controls.iter().any(|c| {
        matches!(
            &c.kind,
            ControlKind::Table(_) | ControlKind::Picture(_)
        )
    });
    if has_text {
        line(out, &format!("PARAGRAPH[id=par-{path}]"));
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
                emit_table(doc, t, out, llm, &ctrl_path);
            }
            ControlKind::Picture(p) => {
                emit_figure(p, out);
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
) {
    let tbl_id = format!("tbl-{path}");
    line(
        out,
        &format!("TABLE[id={tbl_id},rows={},cols={}]", t.rows, t.cols),
    );
    for cell in &t.cells {
        let cell_path = format!("{path}-r{}c{}", cell.row, cell.col);
        emit_cell(doc, cell, out, llm, &cell_path);
    }
    line(out, &format!("END TABLE[{tbl_id}]"));
    out.push('\n');
}

fn emit_cell(
    doc: &IrDocument,
    cell: &TableCell,
    out: &mut String,
    llm: &LlmOptions,
    path: &str,
) {
    let cell_id = format!("cell-{path}");
    let mut header = format!(
        "CELL[id={cell_id},row={},col={},rowspan={},colspan={}",
        cell.row, cell.col, cell.row_span, cell.col_span
    );
    if llm.emit_roles {
        // Classifier not wired yet — always emit `unknown` so downstream
        // consumers can lock onto the attribute without us having to
        // guess. When the heuristic lands it flips per-cell to
        // `label`/`value`/`mixed`.
        header.push_str(",role=unknown");
    }
    if llm.emit_editable {
        header.push_str(",editable=unknown");
    }
    header.push(']');
    line(out, &header);

    for (pi, p) in cell.paragraphs.iter().enumerate() {
        let text = super::markdown::clean_text(&p.text);
        let inner_par_path = format!("{path}-p{pi}");
        if !text.is_empty() {
            line(out, &format!("TEXT[par-{inner_par_path}]: {text}"));
        }
        for (ci, ctrl) in p.controls.iter().enumerate() {
            let inner_ctrl_path = format!("{inner_par_path}-c{ci}");
            match &ctrl.kind {
                ControlKind::Table(nested) => {
                    emit_table(doc, nested, out, llm, &inner_ctrl_path);
                }
                ControlKind::Picture(pic) => {
                    emit_figure(pic, out);
                }
                _ => {}
            }
        }
    }
}

fn emit_figure(pic: &PictureControl, out: &mut String) {
    let fig_id = format!("fig-{}", pic.bin_id);
    let w_mm = hwpunit_to_mm(pic.width_hwpu);
    let h_mm = hwpunit_to_mm(pic.height_hwpu);
    let mut header = format!(
        "FIGURE[id={fig_id},bin_id={},width_mm={w_mm},height_mm={h_mm}",
        pic.bin_id
    );
    if pic.caption_text.is_some() {
        header.push_str(&format!(",caption_ref=cap-{fig_id}"));
    }
    header.push(']');
    line(out, &header);
    if let Some(cap) = pic.caption_text.as_deref() {
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
        assert!(md.contains("PARAGRAPH[id=par-s0-p0]"), "got: {md}");
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
            controls: vec![Control { kind: ControlKind::Table(t) }],
            ..Paragraph::default()
        });
        doc.sections.push(Section { paragraphs: paras, ..Section::default() });
        let md = to_llm_markdown(&doc, &opts_llm());
        assert!(md.contains("TABLE[id=tbl-s0-p3-c0,rows=1,cols=2]"), "got: {md}");
        assert!(
            md.contains("CELL[id=cell-s0-p3-c0-r0c0,row=0,col=0,rowspan=1,colspan=1]"),
            "got: {md}"
        );
        assert!(
            md.contains("CELL[id=cell-s0-p3-c0-r0c1,row=0,col=1,rowspan=1,colspan=1]"),
            "got: {md}"
        );
        assert!(md.contains("TEXT[par-s0-p3-c0-r0c0-p0]: A"), "got: {md}");
        assert!(md.contains("TEXT[par-s0-p3-c0-r0c1-p0]: B"), "got: {md}");
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
                    controls: vec![Control { kind: ControlKind::Table(inner) }],
                    ..Paragraph::default()
                }],
                ..TableCell::default()
            }],
            ..TableControl::default()
        };
        let mut doc = IrDocument::default();
        doc.sections.push(Section {
            paragraphs: vec![Paragraph {
                controls: vec![Control { kind: ControlKind::Table(outer) }],
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
                        caption_text: Some("그림 \u{FFFC}. 시스템 도식".into()),
                    }),
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
                        bin_id: 1, width_hwpu: 0, height_hwpu: 0, caption_text: None,
                    }),
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
    fn role_and_editable_flags_emit_unknown_when_enabled() {
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
                controls: vec![Control { kind: ControlKind::Table(t) }],
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        let opts = MdOptions {
            llm: Some(LlmOptions { emit_roles: true, emit_editable: true }),
            ..MdOptions::default()
        };
        let md = to_llm_markdown(&doc, &opts);
        assert!(md.contains("role=unknown"), "got: {md}");
        assert!(md.contains("editable=unknown"), "got: {md}");
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
        assert!(md.contains("PARAGRAPH[id=par-s0-p0]"));
        assert!(md.contains("PARAGRAPH[id=par-s0-p1]"));
        assert!(md.contains("PARAGRAPH[id=par-s0-p2]"));
    }
}
