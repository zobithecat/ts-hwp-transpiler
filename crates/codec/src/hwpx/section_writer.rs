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

use std::collections::HashMap;

use hwp_transpiler_core::ir::{
    BinaryEntry, CharShapeRun, ControlKind, IrError, Paragraph, PictureControl, Section,
    TableCell, TableControl,
};

/// Bin-id → manifest item stem lookup. Built once at write start
/// from `doc.bin_data` so `<hc:img binaryItemIDRef="...">` can
/// reference the same id the manifest registers (`BIN0001`,
/// `image1`, …) regardless of source format. Without this every
/// picture got `image{N}` regardless of the actual BinData filename
/// — fine for HWPX-sourced docs (their files ARE `imageN.png`)
/// but breaks HWP5-sourced ones (`BIN000N.png`).
type BinLookup = HashMap<u16, String>;

/// Namespace declarations that go on the root `<hs:sec>`. Hancom
/// viewers accept a trimmed set (just `hp` / `hs` / `hc`) in
/// practice; we keep it minimal to reduce bytes-on-disk without
/// risking a schema rejection.
const NS_DECL: &str = concat!(
    r#"xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" "#,
    r#"xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" "#,
    r#"xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core""#,
);

pub fn write_section_xml(section: &Section, bin_data: &[BinaryEntry]) -> Result<Vec<u8>, IrError> {
    let bin_lookup: BinLookup = bin_data
        .iter()
        .filter_map(|e| {
            let stem = e.id.split_once('.').map(|(s, _)| s).unwrap_or(&e.id).to_string();
            entry_bin_id(e).map(|id| (id, stem))
        })
        .collect();

    let mut out = String::new();
    out.push_str(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes" ?>"#);
    out.push_str(&format!("<hs:sec {NS_DECL}>"));

    // Synthetic leading `<hp:p>` carrying `<hp:secPr>` (page size /
    // margins / direction). Real HWPX folds secPr into the first
    // content paragraph's first run; viewers — Hancom HWP 2014
    // included — refuse to render anything when secPr is absent
    // because they have no page geometry to lay text into.
    // Emitting it as a dedicated zero-text paragraph keeps the
    // typed emitter simple and matches what Hancom-authored docs
    // produce when the document opens to a blank state.
    out.push_str(SEC_PR_PARAGRAPH);

    for (i, para) in section.paragraphs.iter().enumerate() {
        // Offset paragraph ids by 1 so the synthetic leading
        // paragraph keeps id=0 to itself.
        emit_paragraph(para, &mut out, (i as u32) + 1, &bin_lookup);
    }

    out.push_str("</hs:sec>");
    Ok(out.into_bytes())
}

/// Default A4-portrait section properties paragraph. Page size and
/// margins mirror Hancom-authored doc fixtures (A4 = 59528 × 84188
/// HWPUNIT, ~25mm side margins). Wrapped in a zero-text `<hp:p>` /
/// `<hp:run>` so it's a valid HWPX prolog without disturbing the
/// content paragraphs that follow.
const SEC_PR_PARAGRAPH: &str = concat!(
    r#"<hp:p id="0" paraPrIDRef="0" styleIDRef="0" pageBreak="0" columnBreak="0" merged="0">"#,
    r#"<hp:run charPrIDRef="0">"#,
    r#"<hp:secPr id="" textDirection="HORIZONTAL" spaceColumns="1134" tabStop="8000" tabStopVal="4000" tabStopUnit="HWPUNIT" outlineShapeIDRef="0" memoShapeIDRef="0" textVerticalWidthHead="0" masterPageCnt="0">"#,
    r#"<hp:grid lineGrid="0" charGrid="0" wonggojiFormat="0"/>"#,
    r#"<hp:startNum pageStartsOn="BOTH" page="0" pic="0" tbl="0" equation="0"/>"#,
    r#"<hp:visibility hideFirstHeader="0" hideFirstFooter="0" hideFirstMasterPage="0" border="SHOW_ALL" fill="SHOW_ALL" hideFirstPageNum="0" hideFirstEmptyLine="0" showLineNumber="0"/>"#,
    r#"<hp:lineNumberShape restartType="0" countBy="0" distance="0" startNumber="0"/>"#,
    r#"<hp:pagePr landscape="WIDELY" width="59528" height="84188" gutterType="LEFT_ONLY">"#,
    r#"<hp:margin header="850" footer="850" gutter="0" left="3000" right="3000" top="1417" bottom="1417"/>"#,
    r#"</hp:pagePr>"#,
    r##"<hp:footNotePr><hp:autoNumFormat type="DIGIT" userChar="" prefixChar="" suffixChar=")" supscript="0"/><hp:noteLine length="-1" type="SOLID" width="0.12 mm" color="#000000"/><hp:noteSpacing betweenNotes="850" belowLine="567" aboveLine="850"/><hp:numbering type="CONTINUOUS" newNum="1"/><hp:placement place="EACH_COLUMN" beneathText="0"/></hp:footNotePr>"##,
    r##"<hp:endNotePr><hp:autoNumFormat type="DIGIT" userChar="" prefixChar="" suffixChar=")" supscript="0"/><hp:noteLine length="-1" type="SOLID" width="0.12 mm" color="#000000"/><hp:noteSpacing betweenNotes="0" belowLine="0" aboveLine="0"/><hp:numbering type="CONTINUOUS" newNum="1"/><hp:placement place="END_OF_DOCUMENT" beneathText="0"/></hp:endNotePr>"##,
    r#"<hp:pageBorderFill type="BOTH" borderFillIDRef="0" textBorder="PAPER" headerInside="0" footerInside="0" fillArea="PAPER"/>"#,
    r#"<hp:pageBorderFill type="EVEN" borderFillIDRef="0" textBorder="PAPER" headerInside="0" footerInside="0" fillArea="PAPER"/>"#,
    r#"<hp:pageBorderFill type="ODD" borderFillIDRef="0" textBorder="PAPER" headerInside="0" footerInside="0" fillArea="PAPER"/>"#,
    r#"</hp:secPr>"#,
    r#"</hp:run>"#,
    r#"<hp:linesegarray><hp:lineseg textpos="0" vertpos="0" vertsize="1000" textheight="1000" baseline="850" spacing="600" horzpos="0" horzsize="42520" flags="393216"/></hp:linesegarray>"#,
    r#"</hp:p>"#,
);

/// Same parser as `asset_pipeline::bin_id_from_entry_id` and the
/// writer's `entry_bin_id`. Kept local to avoid a cross-module
/// dep here; the lookup needs the same numeric `bin_id` HWP5 /
/// HWPX readers store on `PictureControl::bin_id`.
fn entry_bin_id(entry: &BinaryEntry) -> Option<u16> {
    let stem = entry
        .id
        .split_once('.')
        .map(|(s, _)| s)
        .unwrap_or(&entry.id);
    if let Some(rest) = stem.strip_prefix("BIN") {
        return u16::from_str_radix(rest, 16).ok();
    }
    let digits: String = stem
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse::<u16>().ok()
}

/// Render `<hp:linesegarray>` from the paragraph's captured line
/// segments (`PARA_LINE_SEG` ↔ `<hp:lineseg>` is a 1:1 field map).
/// Emits every segment so multi-line paragraphs report their true
/// height. When none were captured the element is omitted entirely:
/// Hancom re-runs line layout for paragraphs without a linesegarray,
/// whereas a synthesised placeholder (vertpos=0) is trusted verbatim
/// and stacks every paragraph at the same position — verified on a
/// real 37-page document, 2026-07-02. `vertpos` is cumulative within
/// its list (body flow / cell), so a stale-but-plausible guess is
/// strictly worse than absence.
fn render_linesegarray(para: &Paragraph, out: &mut String) {
    if para.line_segments.is_empty() {
        return;
    }
    out.push_str("<hp:linesegarray>");
    {
        for s in &para.line_segments {
            out.push_str(&format!(
                r#"<hp:lineseg textpos="{tp}" vertpos="{vp}" vertsize="{vs}" textheight="{th}" baseline="{bl}" spacing="{sp}" horzpos="{hp}" horzsize="{hs}" flags="{fl}"/>"#,
                tp = s.text_start,
                vp = s.vertical_position_hwpu,
                vs = s.line_height_hwpu,
                th = s.text_height_hwpu,
                bl = s.baseline_distance_hwpu,
                sp = s.line_spacing_hwpu,
                hp = s.start_x_hwpu,
                hs = s.width_hwpu,
                fl = s.tag,
            ));
        }
    }
    out.push_str("</hp:linesegarray>");
}

fn emit_paragraph(para: &Paragraph, out: &mut String, id: u32, bin_lookup: &BinLookup) {
    let para_pr = para.header.para_shape_id;
    let style = para.header.style_id;
    out.push_str(&format!(
        r#"<hp:p id="{id}" paraPrIDRef="{para_pr}" styleIDRef="{style}" pageBreak="0" columnBreak="0" merged="0">"#
    ));

    // HWPX paragraph bodies are structured as a sequence of `<hp:run>`s.
    // If the paragraph has char_shape_runs, split at each boundary so
    // each run carries the right charPrIDRef. Controls (tables,
    // pictures) hang off the last run regardless — HWPX allows
    // multiple `<hp:t>` + nested controls within a single run.
    if para.char_shape_runs.is_empty() {
        // No style info captured — emit one run with default style
        // and hang every control off it so nested tables survive.
        emit_run_with_range(para, out, 0, None, para.controls.len(), bin_lookup);
    } else {
        emit_paragraph_as_split_runs(para, out, bin_lookup);
    }

    // `<hp:linesegarray>` — one `<hp:lineseg>` per visual line. When
    // the paragraph carries its real line layout (preserved from the
    // source `PARA_LINE_SEG` record), emit every segment so multi-line
    // paragraphs advance the correct vertical distance. A paragraph
    // that wrapped to N lines but emits a single seed lineseg makes
    // cache-trusting viewers stack all N lines (and the next paragraph)
    // at the same Y — the overlap bug. Falls back to a single Hancom
    // 10pt-on-A4 default only when no layout was captured.
    render_linesegarray(para, out);

    out.push_str("</hp:p>");
}

/// Walk the paragraph's char_shape_runs, emitting one `<hp:run>` per
/// contiguous stretch. The last run also emits any non-text controls
/// (tables, pictures) from `para.controls` — HWPX keeps those inside
/// the run that ends the paragraph, not as paragraph-level siblings.
fn emit_paragraph_as_split_runs(para: &Paragraph, out: &mut String, bin_lookup: &BinLookup) {
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
            bin_lookup,
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
    bin_lookup: &BinLookup,
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

    // Real HWPX puts inline controls (`<hp:pic>` / `<hp:tbl>`) *before*
    // the run's `<hp:t>`. The IR's text carries U+FFFC object-
    // replacement markers as positional placeholders for those
    // controls — they must NOT round-trip into the output, otherwise
    // Hancom viewers render the literal "obj" glyph and the picture
    // ends up double-stamped or visually broken.
    if control_limit > 0 {
        let take = control_limit.min(para.controls.len());
        for ctrl in para.controls.iter().take(take) {
            match &ctrl.kind {
                ControlKind::Table(t) => emit_table(t, out, bin_lookup),
                ControlKind::Picture(p) => emit_picture(p, out, bin_lookup),
                // Equations / unknown gsos round-trip through
                // unknown_streams for now; the writer doesn't know
                // how to reconstruct their XML yet, so they drop
                // silently. Documented gap.
                _ => {}
            }
        }
    }

    let visible_text: String = slice_text.chars().filter(|c| *c != '\u{FFFC}').collect();
    if !visible_text.is_empty() {
        emit_text_with_linebreaks(&visible_text, out);
    }

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

fn emit_table(t: &TableControl, out: &mut String, bin_lookup: &BinLookup) {
    out.push_str(&format!(
        concat!(
            r#"<hp:tbl id="0" zOrder="0" numberingType="TABLE" "#,
            r#"textWrap="TOP_AND_BOTTOM" textFlow="BOTH_SIDES" lock="0" "#,
            r#"dropcapstyle="None" pageBreak="CELL" repeatHeader="1" "#,
            r#"rowCnt="{rows}" colCnt="{cols}" cellSpacing="0" "#,
            r#"borderFillIDRef="{border}" noAdjust="0">"#,
        ),
        rows = t.rows,
        cols = t.cols,
        border = t.border_fill_id,
    ));

    // `<hp:sz>` / `<hp:pos>` / `<hp:outMargin>` / `<hp:inMargin>` —
    // every Hancom-authored `<hp:tbl>` carries these four layout
    // children right after the opening tag. Without them viewers
    // can't determine the table's bounding box or anchor and fall
    // back to broken zero-size rendering. Compute the table extent
    // from cell widths/heights; clamp to 1 HWPU so a degenerate
    // empty table still emits structurally-valid attributes.
    let table_w: u32 = t
        .cells
        .iter()
        .filter(|c| c.row == 0)
        .map(|c| c.width_hwpu)
        .sum::<u32>()
        .max(1);
    let table_h: u32 = (0..t.rows)
        .map(|r| {
            t.cells
                .iter()
                .filter(|c| c.row == r)
                .map(|c| c.height_hwpu)
                .max()
                .unwrap_or(0)
        })
        .sum::<u32>()
        .max(1);
    out.push_str(&format!(
        concat!(
            r#"<hp:sz width="{w}" widthRelTo="ABSOLUTE" height="{h}" heightRelTo="ABSOLUTE" protect="0"/>"#,
            r#"<hp:pos treatAsChar="1" affectLSpacing="0" flowWithText="1" "#,
            r#"allowOverlap="0" holdAnchorAndSO="0" vertRelTo="PARA" horzRelTo="PARA" "#,
            r#"vertAlign="TOP" horzAlign="LEFT" vertOffset="0" horzOffset="0"/>"#,
            r#"<hp:outMargin left="283" right="283" top="283" bottom="283"/>"#,
            r#"<hp:inMargin left="141" right="141" top="141" bottom="141"/>"#,
        ),
        w = table_w,
        h = table_h,
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
            emit_cell(cell, out, bin_lookup);
        }
        out.push_str("</hp:tr>");
    }

    out.push_str("</hp:tbl>");
}

fn emit_cell(cell: &TableCell, out: &mut String, bin_lookup: &BinLookup) {
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
        emit_paragraph(p, out, i as u32, bin_lookup);
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
    // `<hp:cellMargin>` — Hancom-authored cells always carry this.
    // Use IR padding when it's nonzero; otherwise fall back to
    // Hancom's standard 510 (left/right) / 141 (top/bottom) HWPU.
    let pad = cell.padding_hwpu;
    let pad_l = if pad[0] > 0 { pad[0] as i32 } else { 510 };
    let pad_r = if pad[1] > 0 { pad[1] as i32 } else { 510 };
    let pad_t = if pad[2] > 0 { pad[2] as i32 } else { 141 };
    let pad_b = if pad[3] > 0 { pad[3] as i32 } else { 141 };
    out.push_str(&format!(
        r#"<hp:cellMargin left="{l}" right="{r}" top="{t}" bottom="{b}"/>"#,
        l = pad_l,
        r = pad_r,
        t = pad_t,
        b = pad_b,
    ));

    out.push_str("</hp:tc>");
}

/// Emit `<hp:pic>` for a `PictureControl`. The IR carries only the
/// minimum (`bin_id`, `width_hwpu`, `height_hwpu`); we fill in the
/// rest of the children with the defaults observed on Hancom-authored
/// fixtures so the picture renders correctly in HWP / rhwp viewers
/// instead of getting dropped or zero-sized.
fn emit_picture(pic: &PictureControl, out: &mut String, bin_lookup: &BinLookup) {
    let w = pic.width_hwpu.max(1);
    let h = pic.height_hwpu.max(1);
    // Resolve the actual manifest stem (`image1`, `BIN0001`, …)
    // for this picture's bin_id. Falls back to `image{N}` for HWPX-
    // sourced docs whose BinData entries don't appear in the
    // lookup yet (test fixtures with synthetic IRs).
    let bin_ref = bin_lookup
        .get(&pic.bin_id)
        .cloned()
        .unwrap_or_else(|| format!("image{}", pic.bin_id));
    // Each `<hp:pic>` needs a globally-unique `id` — Hancom-authored
    // docs use large random integers (the GSO instance id) and some
    // viewers cross-reference them when laying out multiple pictures
    // on a page. Derive a deterministic non-zero id from `bin_id`
    // (offset to keep small ids out of the `0` collision space and
    // out of the run-charPr / paraPr IDRef numeric range).
    let pic_id: u32 = 1_000_000 + pic.bin_id as u32;
    let instid: u32 = 2_000_000 + pic.bin_id as u32;
    out.push_str(&format!(
        concat!(
            r#"<hp:pic id="{pic_id}" zOrder="0" numberingType="PICTURE" "#,
            r#"textWrap="TOP_AND_BOTTOM" textFlow="BOTH_SIDES" lock="0" "#,
            r#"dropcapstyle="None" href="" groupLevel="0" instid="{instid}" "#,
            r#"reverse="0">"#,
            r#"<hp:offset x="0" y="0"/>"#,
            r#"<hp:orgSz width="{w}" height="{h}"/>"#,
            r#"<hp:curSz width="{w}" height="{h}"/>"#,
            r#"<hp:flip horizontal="0" vertical="0"/>"#,
            r#"<hp:rotationInfo angle="0" centerX="0" centerY="0" rotateimage="1"/>"#,
            r#"<hp:renderingInfo>"#,
            r#"<hc:transMatrix e1="1" e2="0" e3="0" e4="0" e5="1" e6="0"/>"#,
            r#"<hc:scaMatrix e1="1" e2="0" e3="0" e4="0" e5="1" e6="0"/>"#,
            r#"<hc:rotMatrix e1="1" e2="0" e3="0" e4="0" e5="1" e6="0"/>"#,
            r#"</hp:renderingInfo>"#,
            r#"<hc:img binaryItemIDRef="{bin}" bright="0" contrast="0" effect="REAL_PIC" alpha="0"/>"#,
            r#"<hp:imgRect>"#,
            r#"<hc:pt0 x="0" y="0"/>"#,
            r#"<hc:pt1 x="{w}" y="0"/>"#,
            r#"<hc:pt2 x="{w}" y="{h}"/>"#,
            r#"<hc:pt3 x="0" y="{h}"/>"#,
            r#"</hp:imgRect>"#,
            r#"<hp:imgClip left="0" right="{w}" top="0" bottom="{h}"/>"#,
            r#"<hp:inMargin left="0" right="0" top="0" bottom="0"/>"#,
            r#"<hp:imgDim dimwidth="{w}" dimheight="{h}"/>"#,
            r#"<hp:effects/>"#,
            r#"<hp:sz width="{w}" widthRelTo="ABSOLUTE" height="{h}" heightRelTo="ABSOLUTE" protect="0"/>"#,
            r#"<hp:pos treatAsChar="1" affectLSpacing="0" flowWithText="1" "#,
            r#"allowOverlap="1" holdAnchorAndSO="0" vertRelTo="PARA" horzRelTo="PARA" "#,
            r#"vertAlign="TOP" horzAlign="LEFT" vertOffset="0" horzOffset="0"/>"#,
            r#"<hp:outMargin left="0" right="0" top="0" bottom="0"/>"#,
            r#"</hp:pic>"#,
        ),
        w = w,
        h = h,
        bin = bin_ref,
        pic_id = pic_id,
        instid = instid,
    ));
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
        let xml = write_section_xml(&s, &[]).expect("emit");
        let s = std::str::from_utf8(&xml).expect("utf8");
        assert!(s.contains("<hs:sec "));
        assert!(s.contains("</hs:sec>"));
    }

    #[test]
    fn paragraph_emits_hp_p_with_run_and_t() {
        let mut s = Section::default();
        s.paragraphs.push(para_with_text("Hello"));
        let xml = write_section_xml(&s, &[]).expect("emit");
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
        let xml = write_section_xml(&s, &[]).expect("emit");
        let s = std::str::from_utf8(&xml).unwrap();
        assert!(s.contains("A&lt;b&gt; &amp; &quot;quoted&quot; O&#39;Brien"));
        assert!(!s.contains("<b>"));
    }

    #[test]
    fn newlines_become_line_break_elements() {
        let mut s = Section::default();
        s.paragraphs.push(para_with_text("line1\nline2"));
        let xml = write_section_xml(&s, &[]).expect("emit");
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
        let xml = write_section_xml(&s, &[]).expect("emit");
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
        let xml = write_section_xml(&s, &[]).expect("emit");
        let s = std::str::from_utf8(&xml).unwrap();
        assert!(s.contains(r#"colSpan="2" rowSpan="3""#));
    }
}
