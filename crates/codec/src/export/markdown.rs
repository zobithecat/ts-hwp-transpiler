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

use hwp_transpiler_core::ir::{ControlKind, IrDocument, Paragraph, TableCell, TableControl};

pub fn to_markdown(doc: &IrDocument) -> String {
    let mut out = String::new();
    for section in &doc.sections {
        for para in &section.paragraphs {
            emit_paragraph(doc, para, &mut out, 0);
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

fn emit_paragraph(doc: &IrDocument, para: &Paragraph, out: &mut String, depth: usize) {
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
        if let ControlKind::Table(t) = &c.kind {
            emit_table(doc, t, out, depth);
        }
    }
}

fn emit_table(doc: &IrDocument, t: &TableControl, out: &mut String, depth: usize) {
    if is_simple_table(t) {
        emit_md_table(doc, t, out);
    } else {
        emit_table_as_list(doc, t, out, depth);
    }
}

/// A table is "simple" when it maps cleanly to Markdown syntax: rectangular
/// grid, no merged cells, no nested tables, cells contain at most one
/// paragraph (since MD cells are one line).
fn is_simple_table(t: &TableControl) -> bool {
    if t.rows as usize * t.cols as usize != t.cells.len() {
        return false;
    }
    for cell in &t.cells {
        if cell.col_span != 1 || cell.row_span != 1 {
            return false;
        }
        if cell.paragraphs.len() > 1 {
            return false;
        }
        for p in &cell.paragraphs {
            for c in &p.controls {
                if matches!(&c.kind, ControlKind::Table(_)) {
                    return false;
                }
            }
        }
    }
    true
}

fn emit_md_table(doc: &IrDocument, t: &TableControl, out: &mut String) {
    let rows = t.rows as usize;
    let cols = t.cols as usize;
    if rows == 0 || cols == 0 {
        return;
    }
    let mut grid: Vec<Vec<String>> = vec![vec![String::new(); cols]; rows];
    for cell in &t.cells {
        let r = cell.row as usize;
        let c = cell.col as usize;
        if r < rows && c < cols {
            grid[r][c] = md_cell_content(doc, cell);
        }
    }

    // First row as header (Markdown tables always have one).
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

fn md_cell_content(doc: &IrDocument, cell: &TableCell) -> String {
    let mut text = String::new();
    for (i, p) in cell.paragraphs.iter().enumerate() {
        if i > 0 {
            text.push(' ');
        }
        text.push_str(&clean_text(&p.text));
        // Nested tables inside a cell force the complex path — they won't
        // actually reach this function because `is_simple_table` filters
        // them out. But be defensive: if they do, mark them.
        for c in &p.controls {
            if matches!(&c.kind, ControlKind::Table(_)) {
                text.push_str(" [nested-table]");
            }
        }
    }
    // Escape MD table-specific characters.
    text.replace('|', "\\|").replace('\n', " ")
}

fn emit_table_as_list(doc: &IrDocument, t: &TableControl, out: &mut String, depth: usize) {
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
    for cell in &t.cells {
        out.push_str(&indent);
        out.push_str("- ");
        out.push_str(&format!("[{},{}]", cell.row, cell.col));
        if cell.col_span != 1 || cell.row_span != 1 {
            out.push_str(&format!(" span {}×{}", cell.row_span, cell.col_span));
        }
        out.push(':');

        // Inline the cell's paragraphs; nested tables render after at a
        // deeper indent.
        let mut first_line = true;
        let mut sub_tables: Vec<&TableControl> = Vec::new();
        for p in &cell.paragraphs {
            let t_text = clean_text(&p.text);
            for c in &p.controls {
                if let ControlKind::Table(nested) = &c.kind {
                    sub_tables.push(nested);
                }
            }
            if !t_text.is_empty() {
                if first_line {
                    out.push(' ');
                    out.push_str(&t_text);
                    first_line = false;
                } else {
                    out.push_str(&format!("\n{indent}  {t_text}"));
                }
            }
        }
        out.push('\n');

        for nested in sub_tables {
            emit_table(doc, nested, out, depth + 1);
        }
    }
    out.push('\n');
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

fn clean_text(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\u{FFFC}' | '\u{00AD}' => {}
            '\u{00A0}' | '\u{2003}' => out.push(' '),
            _ => out.push(c),
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
    fn ragged_table_emits_bullet_list() {
        let t = TableControl {
            rows: 2,
            cols: 3,
            row_cell_counts: vec![1, 3], // row 0 has 1 merged cell
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
        assert!(md.contains("<!-- table 2×3"));
        assert!(md.contains("- [0,0] span 1×3: merged header"));
        assert!(md.contains("- [1,0]: x"));
        assert!(md.contains("- [1,2]: z"));
    }

    #[test]
    fn cell_pipe_is_escaped() {
        let t = TableControl {
            rows: 1,
            cols: 1,
            row_cell_counts: vec![1],
            cells: vec![cell(0, 0, "a|b")],
            ..TableControl::default()
        };
        let doc = make_doc(vec![style("본문")], vec![para_with_table(t)]);
        let md = to_markdown(&doc);
        assert!(md.contains("| a\\|b |"));
    }
}
