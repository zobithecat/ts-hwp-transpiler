//! HWPX `Contents/section{N}.xml` → IR [`Section`] converter.
//!
//! Recursive-descent over `quick-xml` events. Each parser consumes
//! from the current open tag up to its matching close, building up
//! the IR structure as it goes. Unrecognised elements are skipped
//! (with `skip_until_close`) rather than erroring — HWPX has a wide
//! tag set (headers, footers, tracked-change markers, footnotes,
//! shapes, ...) that a minimum-viable reader doesn't need to support.
//!
//! Scope for v1:
//!   * `<hs:sec>` → `Section`
//!   * `<hp:secPr>` + `<hp:pagePr>` + `<hp:margin>` → `SectionProperties`
//!   * `<hp:p>` → `Paragraph` (text only, no CharShape runs)
//!   * `<hp:run>` → text accumulation + nested `<hp:tbl>` / `<hp:t>`
//!     / `<hp:lineBreak>`
//!   * `<hp:tbl rowCnt colCnt>` + `<hp:tr>` + `<hp:tc>` +
//!     `<hp:cellAddr>` + `<hp:cellSpan>` + `<hp:cellSz>` +
//!     `<hp:subList>` → `TableControl` + `TableCell`
//!
//! Out of scope (surfaced as no-ops, keep data flowing):
//!   * Character / paragraph styles (`charPrIDRef`, `paraPrIDRef`)
//!   * Pictures, equations, shapes, controls
//!   * Line-segment arrays, headers/footers, footnotes
//!   * Tracked-change markers
//!
//! These can be added incrementally without reshaping the parser —
//! just intercept the relevant `<hp:xxx>` Start event and decode.

use std::io::BufRead;

use hwp_transpiler_core::ir::{
    CharShapeRun, Control, ControlKind, IrError, Paragraph, PictureControl, Section,
    SectionProperties, TableCell, TableControl,
};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

/// Entry point. Parses the entire section XML and returns a populated
/// `Section`. Page dimensions land in `section.properties` if the XML
/// carries a `<hp:pagePr>` block (HWPX always does).
pub fn parse_section_xml(xml: &[u8]) -> Result<Section, IrError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);

    let mut section = Section::default();
    let mut buf = Vec::new();

    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err("section", e))?
        {
            Event::Start(e) => match local_name(&e) {
                "sec" => continue,
                "p" => {
                    let para = parse_paragraph(&mut reader, &e, &mut section.properties)?;
                    section.paragraphs.push(para);
                }
                _ => skip_until_close(&mut reader, e.name().as_ref())?,
            },
            Event::Empty(_) => {}
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(section)
}

/// Parse one `<hp:p>` element up to its `</hp:p>`. Collects text from
/// each `<hp:run>` into `Paragraph.text`. A `<hp:secPr>` nested
/// anywhere inside (HWPX convention places it in the first
/// paragraph's first run) promotes its page dimensions into the
/// section's properties — we only write when the field is still zero
/// so earlier paragraphs can't be overridden by later ones.
fn parse_paragraph<R: BufRead>(
    reader: &mut Reader<R>,
    start: &BytesStart,
    section_props: &mut SectionProperties,
) -> Result<Paragraph, IrError> {
    let mut para = Paragraph::default();
    // Wire paraPrIDRef into `para.header.para_shape_id` so the
    // render path can look up ParaShape.align() for text-align CSS.
    para.header.para_shape_id = u32_attr(start, "paraPrIDRef").unwrap_or(0) as u16;
    // Wire styleIDRef into `para.header.style_id` so the human /
    // LLM Markdown exporters' `heading_level` lookup (which keys
    // off `Style::name`) can re-classify headings on round-trip.
    // ParagraphHeader::style_id is a u8 — clamp anything past that.
    para.header.style_id = u32_attr(start, "styleIDRef").unwrap_or(0).min(255) as u8;
    let mut buf = Vec::new();

    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err("paragraph", e))?
        {
            Event::End(e) if local_name_bytes(e.name().as_ref()) == "p" => break,
            Event::Start(e) => match local_name(&e) {
                "run" => {
                    let char_pr_id = u32_attr(&e, "charPrIDRef");
                    parse_run(reader, &mut para, section_props, char_pr_id)?;
                }
                "secPr" => {
                    let props = parse_sec_pr(reader)?;
                    if section_props.page_width_hwpu == 0 {
                        *section_props = props;
                    }
                }
                _ => skip_until_close(reader, e.name().as_ref())?,
            },
            Event::Empty(_) | Event::Text(_) | Event::CData(_) => {}
            Event::Eof => {
                return Err(IrError::Invalid(
                    "unexpected EOF inside <hp:p>".into(),
                ));
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(para)
}

/// Consume a `<hp:run>` body. Each nested `<hp:t>` appends its text
/// to the enclosing paragraph; `<hp:lineBreak/>` becomes a `\n`;
/// `<hp:tbl>` becomes a `ControlKind::Table` on `para.controls`.
/// Any other tag is skipped.
fn parse_run<R: BufRead>(
    reader: &mut Reader<R>,
    para: &mut Paragraph,
    section_props: &mut SectionProperties,
    char_pr_id: Option<u32>,
) -> Result<(), IrError> {
    // Every HWPX run declares a `charPrIDRef` pointing into
    // `doc_info.char_shapes`. Open a shape run at the current UTF-16
    // offset so downstream exporters (MD bold/italic/strike + role
    // classifier's first-shape fingerprint) can resolve it. Runs with
    // no attribute default to id 0 so the exporter still sees
    // *something* rather than an empty `char_shape_runs`.
    let start_u16 = utf16_len(&para.text);
    let shape_id = char_pr_id.unwrap_or(0);
    // Collapse adjacent runs sharing the same shape to avoid boundary
    // noise in paragraphs with many runs of identical styling.
    let last_same = para
        .char_shape_runs
        .last()
        .map(|r| r.char_shape_id == shape_id)
        .unwrap_or(false);
    if !last_same {
        para.char_shape_runs.push(CharShapeRun {
            start: start_u16,
            char_shape_id: shape_id,
        });
    }

    let mut buf = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err("run", e))?
        {
            Event::End(e) if local_name_bytes(e.name().as_ref()) == "run" => break,
            Event::Start(e) => match local_name(&e) {
                "t" => {
                    let text = read_text_content(reader, b"t")?;
                    para.text.push_str(&text);
                }
                "tbl" => {
                    let table = parse_table(reader, &e)?;
                    para.controls.push(Control {
                        kind: ControlKind::Table(table),
                        caption_text: None,
                    });
                }
                "pic" => {
                    let picture = parse_picture(reader, &e)?;
                    para.controls.push(Control {
                        kind: ControlKind::Picture(picture),
                        caption_text: None,
                    });
                }
                "secPr" => {
                    let props = parse_sec_pr(reader)?;
                    if section_props.page_width_hwpu == 0 {
                        *section_props = props;
                    }
                }
                _ => skip_until_close(reader, e.name().as_ref())?,
            },
            Event::Empty(e) => {
                if local_name(&e) == "lineBreak" {
                    para.text.push('\n');
                }
            }
            Event::Eof => {
                return Err(IrError::Invalid(
                    "unexpected EOF inside <hp:run>".into(),
                ));
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

/// Parse one `<hp:pic>` element into a `PictureControl`. HWPX pins
/// the display size on `<hp:curSz width height>` in HWPUNIT and
/// points at the binary with `<hc:img binaryItemIDRef="image1">`.
/// We parse `"image1"` as a decimal bin_id so the HTML emitter's
/// fallback lookup (`image{n}.{ext}`) finds the matching
/// `BinaryEntry`. Everything else (clip / rendering matrix /
/// rotation / etc.) is intentionally skipped for this pass.
fn parse_picture<R: BufRead>(
    reader: &mut Reader<R>,
    _start: &BytesStart,
) -> Result<PictureControl, IrError> {
    let mut pic = PictureControl::default();
    let mut buf = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err("pic", e))?
        {
            Event::End(e) if local_name_bytes(e.name().as_ref()) == "pic" => break,
            Event::Empty(e) => match local_name(&e) {
                "curSz" => {
                    let w = u32_attr(&e, "width").unwrap_or(0);
                    let h = u32_attr(&e, "height").unwrap_or(0);
                    // Many real HWPX docs ship `<hp:curSz width="0"
                    // height="0"/>` and rely on `<hp:orgSz>` for the
                    // actual render dim. Don't overwrite a non-zero
                    // value already pulled from `<hp:orgSz>`.
                    if w > 0 {
                        pic.width_hwpu = w;
                    }
                    if h > 0 {
                        pic.height_hwpu = h;
                    }
                }
                "orgSz" => {
                    // Original / intrinsic picture size, set first so
                    // `<hp:curSz>` only overrides when it carries a
                    // real value.
                    if pic.width_hwpu == 0 {
                        pic.width_hwpu = u32_attr(&e, "width").unwrap_or(0);
                    }
                    if pic.height_hwpu == 0 {
                        pic.height_hwpu = u32_attr(&e, "height").unwrap_or(0);
                    }
                }
                "img" => {
                    if let Some(id_ref) = string_attr(&e, "binaryItemIDRef") {
                        pic.bin_id = parse_hwpx_bin_id(&id_ref);
                    }
                }
                _ => {}
            },
            Event::Start(e) => {
                // `<hp:img>` can appear as a start+end pair in some
                // HWPX variants — handle that shape too so the bin_id
                // is still captured.
                if local_name(&e) == "img" {
                    if let Some(id_ref) = string_attr(&e, "binaryItemIDRef") {
                        pic.bin_id = parse_hwpx_bin_id(&id_ref);
                    }
                }
                skip_until_close(reader, e.name().as_ref())?;
            }
            Event::Eof => {
                return Err(IrError::Invalid(
                    "unexpected EOF inside <hp:pic>".into(),
                ));
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(pic)
}

/// `"image12"` → `12`. Strips the literal `image` prefix that HWPX
/// uses for `binaryItemIDRef` and parses the remainder as a decimal.
/// Falls back to 0 for unrecognised ids — the render path checks
/// whether the binary entry actually exists before emitting `<img>`.
fn parse_hwpx_bin_id(s: &str) -> u16 {
    s.strip_prefix("image")
        .and_then(|n| n.parse::<u16>().ok())
        .unwrap_or(0)
}

fn string_attr(e: &BytesStart<'_>, name: &str) -> Option<String> {
    for attr in e.attributes().flatten() {
        if attr.key.as_ref() == name.as_bytes() {
            return std::str::from_utf8(&attr.value).ok().map(|s| s.to_string());
        }
    }
    None
}

/// Parse one `<hp:tbl>` element. `rowCnt` / `colCnt` come from its
/// attributes; the body contains `<hp:tr>` → `<hp:tc>` → `<hp:subList>`
/// → nested paragraphs. `cellAddr` / `cellSpan` / `cellSz` sit as
/// siblings of `subList` inside each `<hp:tc>`.
fn parse_table<R: BufRead>(
    reader: &mut Reader<R>,
    start: &BytesStart,
) -> Result<TableControl, IrError> {
    let rows = u32_attr(start, "rowCnt").unwrap_or(0) as u16;
    let cols = u32_attr(start, "colCnt").unwrap_or(0) as u16;

    let mut table = TableControl {
        rows,
        cols,
        ..TableControl::default()
    };

    let mut buf = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err("table", e))?
        {
            Event::End(e) if local_name_bytes(e.name().as_ref()) == "tbl" => break,
            Event::Start(e) => match local_name(&e) {
                "tr" => parse_row(reader, &mut table)?,
                // `<hp:tc>` can sit directly under `<hp:tbl>` in some
                // variants (no `<hp:tr>` wrapper). Accept both forms.
                "tc" => {
                    let cell = parse_cell(reader, &e)?;
                    table.cells.push(cell);
                }
                _ => skip_until_close(reader, e.name().as_ref())?,
            },
            Event::Empty(_) | Event::Text(_) | Event::CData(_) => {}
            Event::Eof => {
                return Err(IrError::Invalid(
                    "unexpected EOF inside <hp:tbl>".into(),
                ));
            }
            _ => {}
        }
        buf.clear();
    }

    // `row_cell_counts` isn't surfaced by HWPX (it's an HWP5-only
    // writer concern). Leave as empty — the MD/HTML exporters handle
    // that case via `t.cells` directly.
    Ok(table)
}

fn parse_row<R: BufRead>(
    reader: &mut Reader<R>,
    table: &mut TableControl,
) -> Result<(), IrError> {
    let mut buf = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err("tr", e))?
        {
            Event::End(e) if local_name_bytes(e.name().as_ref()) == "tr" => break,
            Event::Start(e) => match local_name(&e) {
                "tc" => {
                    let cell = parse_cell(reader, &e)?;
                    table.cells.push(cell);
                }
                _ => skip_until_close(reader, e.name().as_ref())?,
            },
            Event::Empty(_) | Event::Text(_) | Event::CData(_) => {}
            Event::Eof => {
                return Err(IrError::Invalid(
                    "unexpected EOF inside <hp:tr>".into(),
                ));
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

fn parse_cell<R: BufRead>(
    reader: &mut Reader<R>,
    start: &BytesStart,
) -> Result<TableCell, IrError> {
    // `borderFillIDRef` on `<hp:tc>` drives the role classifier's bg-
    // tone fingerprint. Without it every HWPX cell would fingerprint
    // as `BgTone::None` and the classifier would fall through to its
    // position-only heuristic, labelling everything `value`.
    let border_fill_id = u32_attr(start, "borderFillIDRef").unwrap_or(0) as u16;
    let mut cell = TableCell {
        col_span: 1,
        row_span: 1,
        border_fill_id,
        ..TableCell::default()
    };
    let mut buf = Vec::new();

    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err("tc", e))?
        {
            Event::End(e) if local_name_bytes(e.name().as_ref()) == "tc" => break,
            Event::Start(e) => match local_name(&e) {
                "subList" => parse_cell_paragraphs(reader, &mut cell)?,
                _ => skip_until_close(reader, e.name().as_ref())?,
            },
            Event::Empty(e) => match local_name(&e) {
                "cellAddr" => {
                    cell.col = u32_attr(&e, "colAddr").unwrap_or(0) as u16;
                    cell.row = u32_attr(&e, "rowAddr").unwrap_or(0) as u16;
                }
                "cellSpan" => {
                    cell.col_span = u32_attr(&e, "colSpan").unwrap_or(1) as u16;
                    cell.row_span = u32_attr(&e, "rowSpan").unwrap_or(1) as u16;
                }
                "cellSz" => {
                    cell.width_hwpu = u32_attr(&e, "width").unwrap_or(0);
                    cell.height_hwpu = u32_attr(&e, "height").unwrap_or(0);
                }
                _ => {}
            },
            Event::Text(_) | Event::CData(_) => {}
            Event::Eof => {
                return Err(IrError::Invalid(
                    "unexpected EOF inside <hp:tc>".into(),
                ));
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(cell)
}

fn parse_cell_paragraphs<R: BufRead>(
    reader: &mut Reader<R>,
    cell: &mut TableCell,
) -> Result<(), IrError> {
    let mut buf = Vec::new();
    // Cells never carry `<hp:secPr>` in practice; give parse_paragraph
    // a throwaway slot so its signature is satisfied.
    let mut scratch = SectionProperties::default();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err("subList", e))?
        {
            Event::End(e) if local_name_bytes(e.name().as_ref()) == "subList" => break,
            Event::Start(e) => match local_name(&e) {
                "p" => {
                    let para = parse_paragraph(reader, &e, &mut scratch)?;
                    cell.paragraphs.push(para);
                }
                _ => skip_until_close(reader, e.name().as_ref())?,
            },
            Event::Empty(_) | Event::Text(_) | Event::CData(_) => {}
            Event::Eof => {
                return Err(IrError::Invalid(
                    "unexpected EOF inside <hp:subList>".into(),
                ));
            }
            _ => {}
        }
        buf.clear();
    }
    cell.para_count = cell.paragraphs.len() as i32;
    Ok(())
}

/// Extract page dimensions + margins from a `<hp:secPr>` body. The
/// dims live on `<hp:pagePr>` (an empty element with `width` /
/// `height` attrs) plus its child `<hp:margin>` (left / right / top /
/// bottom). Everything else in `<hp:secPr>` is ignored.
fn parse_sec_pr<R: BufRead>(reader: &mut Reader<R>) -> Result<SectionProperties, IrError> {
    let mut props = SectionProperties::default();
    let mut buf = Vec::new();
    let mut depth = 1;

    while depth > 0 {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err("secPr", e))?
        {
            Event::End(e) if local_name_bytes(e.name().as_ref()) == "secPr" => {
                depth -= 1;
            }
            Event::Start(e) => {
                // pagePr sometimes appears as non-empty with margin child.
                let name = local_name(&e);
                if name == "pagePr" {
                    props.page_width_hwpu = u32_attr(&e, "width").unwrap_or(0);
                    props.page_height_hwpu = u32_attr(&e, "height").unwrap_or(0);
                    // Fall through — margin comes next as an Empty event.
                } else {
                    depth += 1;
                }
            }
            Event::Empty(e) => match local_name(&e) {
                "pagePr" => {
                    props.page_width_hwpu = u32_attr(&e, "width").unwrap_or(0);
                    props.page_height_hwpu = u32_attr(&e, "height").unwrap_or(0);
                }
                "margin" => {
                    props.margins_hwpu = [
                        u32_attr(&e, "top").unwrap_or(0),
                        u32_attr(&e, "right").unwrap_or(0),
                        u32_attr(&e, "bottom").unwrap_or(0),
                        u32_attr(&e, "left").unwrap_or(0),
                    ];
                }
                _ => {}
            },
            Event::End(_) => {
                depth -= 1;
            }
            Event::Eof => {
                return Err(IrError::Invalid(
                    "unexpected EOF inside <hp:secPr>".into(),
                ));
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(props)
}

/// Consume events until we hit the matching End tag. Handles nesting
/// of same-named elements by counting depth. Used for any Start event
/// whose contents we don't care about.
fn skip_until_close<R: BufRead>(
    reader: &mut Reader<R>,
    end_name: &[u8],
) -> Result<(), IrError> {
    let mut depth = 1;
    let mut buf = Vec::new();
    while depth > 0 {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err("skip", e))?
        {
            Event::Start(e) if e.name().as_ref() == end_name => depth += 1,
            Event::End(e) if e.name().as_ref() == end_name => depth -= 1,
            Event::Eof => {
                return Err(IrError::Invalid(format!(
                    "EOF while skipping {}",
                    String::from_utf8_lossy(end_name)
                )));
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

/// Local-part (prefix-stripped) of an XML tag name. HWPX uses a
/// handful of namespace prefixes (`hp:`, `hs:`, `hc:`, `hh:`, ...);
/// the local part is what carries the semantic distinction. Returned
/// string borrows from the `BytesStart`'s underlying buffer so no
/// allocation is needed on the hot parse path.
fn local_name<'a>(e: &'a BytesStart<'_>) -> &'a str {
    local_name_bytes(e.name().into_inner())
}

fn local_name_bytes(bytes: &[u8]) -> &str {
    let start = bytes.iter().position(|&b| b == b':').map(|i| i + 1).unwrap_or(0);
    std::str::from_utf8(&bytes[start..]).unwrap_or("")
}

/// Consume events inside a simple text-bearing element (`<hp:t>`)
/// until the matching end tag and return the accumulated text.
/// Handles Text, CData, and entity references; re-enters the parser
/// on nested non-text elements (treated as plain passthrough —
/// `<hp:t>` doesn't legitimately nest structural children).
fn read_text_content<R: BufRead>(
    reader: &mut Reader<R>,
    local_end: &[u8],
) -> Result<String, IrError> {
    let mut out = String::new();
    let mut buf = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err("text", e))?
        {
            Event::Text(t) => {
                let s = t
                    .unescape()
                    .map_err(|e| xml_err("text unescape", e))?;
                out.push_str(&s);
            }
            Event::CData(c) => {
                out.push_str(&String::from_utf8_lossy(c.as_ref()));
            }
            Event::End(e) if local_name_bytes(e.name().as_ref()).as_bytes() == local_end => {
                break;
            }
            Event::Eof => {
                return Err(IrError::Invalid(format!(
                    "EOF in text element <{}>",
                    std::str::from_utf8(local_end).unwrap_or("?")
                )));
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

/// Parse a numeric attribute. HWPX attributes for sizes and counts
/// are always decimal integers; non-integer values are treated as
/// missing.
fn u32_attr(e: &BytesStart<'_>, name: &str) -> Option<u32> {
    for attr in e.attributes().flatten() {
        if attr.key.as_ref() == name.as_bytes() {
            let v = std::str::from_utf8(&attr.value).ok()?;
            return v.trim().parse::<u32>().ok();
        }
    }
    None
}

fn xml_err(context: &str, e: quick_xml::Error) -> IrError {
    IrError::Invalid(format!("hwpx xml ({context}): {e}"))
}

/// UTF-16 code-unit length of a string — matches the coordinate
/// system HWP uses for `CharShapeRun::start` offsets. Sum is cheap:
/// each char contributes 1 or 2 code units.
fn utf16_len(s: &str) -> u32 {
    s.chars().map(|c| c.len_utf16() as u32).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_section_parses_to_empty() {
        let xml = r#"<?xml version="1.0"?><hs:sec xmlns:hs="x"/>"#;
        let s = parse_section_xml(xml.as_bytes()).expect("parse");
        assert_eq!(s.paragraphs.len(), 0);
    }

    #[test]
    fn one_paragraph_with_text_run() {
        let xml = r#"<?xml version="1.0"?>
            <hs:sec xmlns:hs="x" xmlns:hp="y">
              <hp:p id="1" paraPrIDRef="0" styleIDRef="0" pageBreak="0" columnBreak="0" merged="0">
                <hp:run charPrIDRef="0"><hp:t>Hello HWPX</hp:t></hp:run>
              </hp:p>
            </hs:sec>"#;
        let s = parse_section_xml(xml.as_bytes()).expect("parse");
        assert_eq!(s.paragraphs.len(), 1);
        assert_eq!(s.paragraphs[0].text, "Hello HWPX");
    }

    #[test]
    fn multiple_runs_concatenate_text() {
        let xml = r#"<?xml version="1.0"?>
            <hs:sec xmlns:hp="y">
              <hp:p>
                <hp:run><hp:t>창업</hp:t></hp:run>
                <hp:run><hp:t>도약패키지</hp:t></hp:run>
              </hp:p>
            </hs:sec>"#;
        let s = parse_section_xml(xml.as_bytes()).expect("parse");
        assert_eq!(s.paragraphs[0].text, "창업도약패키지");
    }

    #[test]
    fn line_break_becomes_newline() {
        let xml = r#"<?xml version="1.0"?>
            <hs:sec xmlns:hp="y">
              <hp:p><hp:run><hp:t>A</hp:t><hp:lineBreak/><hp:t>B</hp:t></hp:run></hp:p>
            </hs:sec>"#;
        let s = parse_section_xml(xml.as_bytes()).expect("parse");
        assert_eq!(s.paragraphs[0].text, "A\nB");
    }

    #[test]
    fn one_by_one_table_with_cell_text() {
        let xml = r#"<?xml version="1.0"?>
            <hs:sec xmlns:hp="y">
              <hp:p><hp:run><hp:tbl rowCnt="1" colCnt="1">
                <hp:tr><hp:tc>
                  <hp:subList><hp:p><hp:run><hp:t>cell</hp:t></hp:run></hp:p></hp:subList>
                  <hp:cellAddr colAddr="0" rowAddr="0"/>
                  <hp:cellSpan colSpan="1" rowSpan="1"/>
                  <hp:cellSz width="100" height="50"/>
                </hp:tc></hp:tr>
              </hp:tbl></hp:run></hp:p>
            </hs:sec>"#;
        let s = parse_section_xml(xml.as_bytes()).expect("parse");
        let ctrl = &s.paragraphs[0].controls[0];
        let ControlKind::Table(t) = &ctrl.kind else {
            panic!("expected Table control");
        };
        assert_eq!(t.rows, 1);
        assert_eq!(t.cols, 1);
        assert_eq!(t.cells.len(), 1);
        assert_eq!(t.cells[0].paragraphs[0].text, "cell");
        assert_eq!(t.cells[0].width_hwpu, 100);
        assert_eq!(t.cells[0].height_hwpu, 50);
    }

    #[test]
    fn table_cell_span_and_addr_decoded() {
        let xml = r#"<?xml version="1.0"?>
            <hs:sec xmlns:hp="y">
              <hp:p><hp:run><hp:tbl rowCnt="2" colCnt="2">
                <hp:tr><hp:tc>
                  <hp:subList><hp:p><hp:run><hp:t>merged</hp:t></hp:run></hp:p></hp:subList>
                  <hp:cellAddr colAddr="0" rowAddr="0"/>
                  <hp:cellSpan colSpan="2" rowSpan="1"/>
                  <hp:cellSz width="200" height="50"/>
                </hp:tc></hp:tr>
              </hp:tbl></hp:run></hp:p>
            </hs:sec>"#;
        let s = parse_section_xml(xml.as_bytes()).expect("parse");
        let ControlKind::Table(t) = &s.paragraphs[0].controls[0].kind else {
            panic!()
        };
        assert_eq!(t.cells[0].col_span, 2);
        assert_eq!(t.cells[0].row_span, 1);
    }

    #[test]
    fn sec_pr_populates_page_dims() {
        // A4 portrait: 59528 × 84168 HWPUNIT, 20mm / 15mm / 20mm / 15mm margins.
        let xml = r#"<?xml version="1.0"?>
            <hs:sec xmlns:hp="y">
              <hp:p>
                <hp:run>
                  <hp:secPr>
                    <hp:pagePr width="59528" height="84168">
                      <hp:margin top="5669" right="4251" bottom="5669" left="4251"/>
                    </hp:pagePr>
                  </hp:secPr>
                </hp:run>
              </hp:p>
            </hs:sec>"#;
        let s = parse_section_xml(xml.as_bytes()).expect("parse");
        assert_eq!(s.properties.page_width_hwpu, 59528);
        assert_eq!(s.properties.page_height_hwpu, 84168);
        assert_eq!(s.properties.margins_hwpu, [5669, 4251, 5669, 4251]);
    }

    #[test]
    fn unknown_element_is_skipped() {
        let xml = r#"<?xml version="1.0"?>
            <hs:sec xmlns:hp="y">
              <hp:p>
                <hp:unknown>ignored<hp:nested>also</hp:nested></hp:unknown>
                <hp:run><hp:t>kept</hp:t></hp:run>
              </hp:p>
            </hs:sec>"#;
        let s = parse_section_xml(xml.as_bytes()).expect("parse");
        assert_eq!(s.paragraphs[0].text, "kept");
    }
}
