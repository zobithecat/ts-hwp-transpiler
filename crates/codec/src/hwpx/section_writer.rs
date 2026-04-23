//! IR [`Section`] → `Contents/section{N}.xml` serializer.
//!
//! Roundtrip is **structural**, not byte-equal: the reader throws
//! away layout details it doesn't care about (linesegarrays,
//! positional metadata on tables, style refs not captured), so the
//! writer can't reproduce the exact original bytes even for an
//! untouched doc. The goal is "emit a file any HWPX viewer will
//! open", which the emitted structure meets.
//!
//! Emitted tree, minimum shape per element:
//!
//!   <hs:sec xmlns:hp="…" xmlns:hs="…" xmlns:hc="…">
//!     <hp:p id="{n}" paraPrIDRef="0" styleIDRef="0"
//!           pageBreak="0" columnBreak="0" merged="0">
//!       <hp:run charPrIDRef="{cs}">
//!         <hp:t>text</hp:t>
//!         <hp:lineBreak/>              (on embedded '\n')
//!         <hp:tbl rowCnt colCnt …>…</hp:tbl>
//!       </hp:run>
//!     </hp:p>
//!   </hs:sec>
//!
//! Style refs default to 0 because our reader never captured
//! paraPrIDRef; char_shape_runs that survived round-trip do reach
//! the output here. Tables carry enough attributes for viewers to
//! accept them (`zOrder`, `numberingType`, `textWrap`, …) defaulted
//! to the values we've observed in Hancom-written fixtures.

use hwp_transpiler_core::ir::{
    CharShapeRun, ControlKind, IrError, Paragraph, Section, TableCell, TableControl,
};

/// Namespace declarations that go on the root `<hs:sec>`. Hancom
/// viewers accept a trimmed set (just `hp` / `hs` / `hc`) in
/// practice; we keep it minimal to reduce bytes-on-disk without
/// risking a schema rejection.
const NS_DECL: &str = concat!(
    r#"xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" "#,
    r#"xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" "#,
    r#"xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core""#,
);

pub fn write_section_xml(section: &Section) -> Result<Vec<u8>, IrError> {
    let mut out = String::new();
    out.push_str(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes" ?>"#);
    out.push_str(&format!("<hs:sec {NS_DECL}>"));

    for (i, para) in section.paragraphs.iter().enumerate() {
        emit_paragraph(para, &mut out, i as u32);
    }

    out.push_str("</hs:sec>");
    Ok(out.into_bytes())
}

fn emit_paragraph(para: &Paragraph, out: &mut String, id: u32) {
    out.push_str(&format!(
        r#"<hp:p id="{id}" paraPrIDRef="0" styleIDRef="0" pageBreak="0" columnBreak="0" merged="0">"#
    ));

    // HWPX paragraph bodies are structured as a sequence of `<hp:run>`s.
    // If the paragraph has char_shape_runs, split at each boundary so
    // each run carries the right charPrIDRef. Controls (tables,
    // pictures) hang off the last run regardless — HWPX allows
    // multiple `<hp:t>` + nested controls within a single run.
    if para.char_shape_runs.is_empty() {
        // No style info captured — emit one run with default style
        // and hang every control off it so nested tables survive.
        emit_run_with_range(para, out, 0, None, para.controls.len());
    } else {
        emit_paragraph_as_split_runs(para, out);
    }

    out.push_str("</hp:p>");
}

/// Walk the paragraph's char_shape_runs, emitting one `<hp:run>` per
/// contiguous stretch. The last run also emits any non-text controls
/// (tables, pictures) from `para.controls` — HWPX keeps those inside
/// the run that ends the paragraph, not as paragraph-level siblings.
fn emit_paragraph_as_split_runs(para: &Paragraph, out: &mut String) {
    let text_u16: Vec<u16> = para.text.encode_utf16().collect();
    let total = text_u16.len() as u32;
    for (i, run) in para.char_shape_runs.iter().enumerate() {
        let end = para
            .char_shape_runs
            .get(i + 1)
            .map(|r| r.start)
            .unwrap_or(total);
        let is_last = i + 1 == para.char_shape_runs.len();
        emit_run_with_range(
            para,
            out,
            run.char_shape_id,
            Some((run.start, end, &text_u16)),
            if is_last { para.controls.len() } else { 0 },
        );
    }
}

/// Emit a single `<hp:run>`. `range` is the `(start, end, full_utf16)`
/// tuple when we're splitting the paragraph across multiple shape
/// runs; `None` means the run covers the entire paragraph text.
/// `control_limit` bounds how many of `para.controls` this run takes
/// — nonzero only on the final run of a paragraph so controls
/// aren't emitted multiple times.
fn emit_run_with_range(
    para: &Paragraph,
    out: &mut String,
    char_shape_id: u32,
    range: Option<(u32, u32, &[u16])>,
    control_limit: usize,
) {
    out.push_str(&format!(r#"<hp:run charPrIDRef="{char_shape_id}">"#));

    let slice_text: String = match range {
        Some((start, end, full_utf16)) => {
            let s = start as usize;
            let e = (end as usize).min(full_utf16.len());
            if s < e {
                String::from_utf16_lossy(&full_utf16[s..e])
            } else {
                String::new()
            }
        }
        None => para.text.clone(),
    };

    emit_text_with_linebreaks(&slice_text, out);

    if control_limit > 0 {
        let take = control_limit.min(para.controls.len());
        for ctrl in para.controls.iter().take(take) {
            match &ctrl.kind {
                ControlKind::Table(t) => emit_table(t, out),
                // Pictures and other gsos round-trip through
                // unknown_streams for now; the writer doesn't know
                // how to reconstruct their XML yet, so they drop
                // silently. Documented gap.
                _ => {}
            }
        }
    } else if para.controls.is_empty() {
        // no-op — no controls at all
    }
    // When `control_limit == 0` but there *are* controls on the
    // paragraph, we expect the caller to have attached them to a
    // later run.

    out.push_str("</hp:run>");
}

/// Split on `\n` boundaries, emitting `<hp:lineBreak/>` between
/// segments. Matches HWPX convention where in-paragraph line breaks
/// are their own self-closing element rather than embedded whitespace.
fn emit_text_with_linebreaks(text: &str, out: &mut String) {
    let mut first = true;
    for part in text.split('\n') {
        if !first {
            out.push_str("<hp:lineBreak/>");
        }
        first = false;
        if !part.is_empty() {
            out.push_str("<hp:t>");
            out.push_str(&escape_xml(part));
            out.push_str("</hp:t>");
        }
    }
}

fn emit_table(t: &TableControl, out: &mut String) {
    out.push_str(&format!(
        concat!(
            r#"<hp:tbl id="0" zOrder="0" numberingType="TABLE" "#,
            r#"textWrap="TOP_AND_BOTTOM" textFlow="BOTH_SIDES" lock="0" "#,
            r#"dropcapstyle="None" pageBreak="CELL" repeatHeader="1" "#,
            r#"rowCnt="{rows}" colCnt="{cols}" cellSpacing="0" "#,
            r#"borderFillIDRef="0" noAdjust="0">"#,
        ),
        rows = t.rows,
        cols = t.cols,
    ));

    // Group cells by row-index into `<hp:tr>` wrappers so the output
    // is predictable for viewers that require the wrapper. Cells
    // without a direct row match still live inside one of the row
    // groups — IR cells always carry an explicit `row`/`col`.
    for r in 0..t.rows {
        let mut row_cells: Vec<&TableCell> =
            t.cells.iter().filter(|c| c.row == r).collect();
        row_cells.sort_by_key(|c| c.col);
        if row_cells.is_empty() {
            continue;
        }
        out.push_str("<hp:tr>");
        for cell in row_cells {
            emit_cell(cell, out);
        }
        out.push_str("</hp:tr>");
    }

    out.push_str("</hp:tbl>");
}

fn emit_cell(cell: &TableCell, out: &mut String) {
    out.push_str(&format!(
        concat!(
            r#"<hp:tc name="" header="0" hasMargin="0" protect="0" "#,
            r#"editable="0" dirty="0" borderFillIDRef="{border}">"#,
        ),
        border = cell.border_fill_id,
    ));

    // Cell paragraphs live inside `<hp:subList>`. Reuse the top-
    // level paragraph emitter so cell bodies share the exact same
    // rules (runs, linebreaks, nested tables).
    out.push_str(
        r#"<hp:subList id="" textDirection="HORIZONTAL" lineWrap="BREAK" vertAlign="CENTER" linkListIDRef="0" linkListNextIDRef="0" textWidth="0" textHeight="0" hasTextRef="0" hasNumRef="0">"#,
    );
    for (i, p) in cell.paragraphs.iter().enumerate() {
        emit_paragraph(p, out, i as u32);
    }
    out.push_str("</hp:subList>");

    out.push_str(&format!(
        r#"<hp:cellAddr colAddr="{c}" rowAddr="{r}"/>"#,
        c = cell.col,
        r = cell.row,
    ));
    out.push_str(&format!(
        r#"<hp:cellSpan colSpan="{cs}" rowSpan="{rs}"/>"#,
        cs = cell.col_span,
        rs = cell.row_span,
    ));
    out.push_str(&format!(
        r#"<hp:cellSz width="{w}" height="{h}"/>"#,
        w = cell.width_hwpu,
        h = cell.height_hwpu,
    ));

    out.push_str("</hp:tc>");
}

fn escape_xml(s: &str) -> String {
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

// `CharShapeRun` is used transitively via Paragraph fields; silence
// the "unused import" noise by taking a reference once.
#[allow(dead_code)]
fn _type_hint(_r: &CharShapeRun) {}

#[cfg(test)]
mod tests {
    use super::*;
    use hwp_transpiler_core::ir::{Control, ControlKind, Paragraph, Section, TableCell, TableControl};

    fn para_with_text(text: &str) -> Paragraph {
        Paragraph {
            text: text.into(),
            ..Paragraph::default()
        }
    }

    #[test]
    fn empty_section_wraps_in_sec_element() {
        let s = Section::default();
        let xml = write_section_xml(&s).expect("emit");
        let s = std::str::from_utf8(&xml).expect("utf8");
        assert!(s.contains("<hs:sec "));
        assert!(s.contains("</hs:sec>"));
    }

    #[test]
    fn paragraph_emits_hp_p_with_run_and_t() {
        let mut s = Section::default();
        s.paragraphs.push(para_with_text("Hello"));
        let xml = write_section_xml(&s).expect("emit");
        let s = std::str::from_utf8(&xml).unwrap();
        assert!(s.contains("<hp:p "));
        assert!(s.contains(r#"<hp:run charPrIDRef="0">"#));
        assert!(s.contains("<hp:t>Hello</hp:t>"));
    }

    #[test]
    fn special_chars_are_escaped() {
        let mut s = Section::default();
        s.paragraphs
            .push(para_with_text(r#"A<b> & "quoted" O'Brien"#));
        let xml = write_section_xml(&s).expect("emit");
        let s = std::str::from_utf8(&xml).unwrap();
        assert!(s.contains("A&lt;b&gt; &amp; &quot;quoted&quot; O&#39;Brien"));
        assert!(!s.contains("<b>"));
    }

    #[test]
    fn newlines_become_line_break_elements() {
        let mut s = Section::default();
        s.paragraphs.push(para_with_text("line1\nline2"));
        let xml = write_section_xml(&s).expect("emit");
        let s = std::str::from_utf8(&xml).unwrap();
        assert!(s.contains("<hp:t>line1</hp:t><hp:lineBreak/><hp:t>line2</hp:t>"));
    }

    #[test]
    fn table_emits_tbl_tr_tc_subList_cellAddr() {
        let cell = TableCell {
            col: 0,
            row: 0,
            col_span: 1,
            row_span: 1,
            width_hwpu: 100,
            height_hwpu: 50,
            paragraphs: vec![para_with_text("cell")],
            ..TableCell::default()
        };
        let t = TableControl {
            rows: 1,
            cols: 1,
            cells: vec![cell],
            ..TableControl::default()
        };
        let mut s = Section::default();
        s.paragraphs.push(Paragraph {
            controls: vec![Control {
                kind: ControlKind::Table(t),
                ..Default::default()
            }],
            ..Paragraph::default()
        });
        let xml = write_section_xml(&s).expect("emit");
        let s = std::str::from_utf8(&xml).unwrap();
        assert!(s.contains(r#"<hp:tbl id="0""#));
        assert!(s.contains(r#"rowCnt="1" colCnt="1""#));
        assert!(s.contains("<hp:tr>"));
        assert!(s.contains(r#"<hp:tc "#));
        assert!(s.contains(r#"<hp:subList "#));
        assert!(s.contains("<hp:t>cell</hp:t>"));
        assert!(s.contains(r#"<hp:cellAddr colAddr="0" rowAddr="0"/>"#));
        assert!(s.contains(r#"<hp:cellSpan colSpan="1" rowSpan="1"/>"#));
        assert!(s.contains(r#"<hp:cellSz width="100" height="50"/>"#));
    }

    #[test]
    fn merged_cell_spans_survive() {
        let cell = TableCell {
            col: 0,
            row: 0,
            col_span: 2,
            row_span: 3,
            paragraphs: vec![para_with_text("m")],
            ..TableCell::default()
        };
        let t = TableControl {
            rows: 3,
            cols: 2,
            cells: vec![cell],
            ..TableControl::default()
        };
        let mut s = Section::default();
        s.paragraphs.push(Paragraph {
            controls: vec![Control {
                kind: ControlKind::Table(t),
                ..Default::default()
            }],
            ..Paragraph::default()
        });
        let xml = write_section_xml(&s).expect("emit");
        let s = std::str::from_utf8(&xml).unwrap();
        assert!(s.contains(r#"colSpan="2" rowSpan="3""#));
    }
}
