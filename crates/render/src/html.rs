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
    for (si, section) in doc.sections.iter().enumerate() {
        emit_section_open(section, &mut out, opts, si);
        // Two overlapping stacks here:
        //
        //   * `depth` drives visual indentation — the current
        //     heading level, used to set `padding-left` on content.
        //   * `chapter_stack` wraps logical chapters in nested
        //     `<section>` elements so the tree reads as a proper
        //     document outline (accessible + anchor-friendly).
        //     Each entry remembers the heading level that opened it
        //     so we know when to close at `emit_heading_at_level`.
        let mut depth: u8 = 0;
        let mut chapter_stack: Vec<u8> = Vec::new();
        for (pi, para) in section.paragraphs.iter().enumerate() {
            let path = format!("s{si}-p{pi}");
            if let Some(level) = heading_level(doc, para) {
                // Close any chapter sections that are same-level or
                // deeper — their scope is ending.
                while chapter_stack
                    .last()
                    .map(|&l| l >= level)
                    .unwrap_or(false)
                {
                    chapter_stack.pop();
                    out.push_str("</section>\n");
                }
                // Open a new chapter wrapper.
                out.push_str(&format!(
                    r#"<section class="hwp-chapter hwp-lv-{level}" id="sec-{path}">"#
                ));
                out.push('\n');
                chapter_stack.push(level);
                depth = level;
            }
            emit_paragraph(doc, para, &mut out, opts, depth, &path);
        }
        // Close any chapters still open at section end.
        while chapter_stack.pop().is_some() {
            out.push_str("</section>\n");
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

fn emit_section_open(
    section: &Section,
    out: &mut String,
    opts: &HtmlOptions,
    si: usize,
) {
    let id = format!(r#" id="sec-{si}""#);
    if opts.emit_pages {
        // Splice the id into the page-dim tag. `page_open_tag`
        // already emits `<section class="hwp-page" style="...">`;
        // inject the id before `class=` so attribute order reads
        // naturally.
        let tag = page_open_tag(&section.properties);
        out.push_str(&tag.replacen("<section", &format!("<section{id}"), 1));
    } else {
        out.push_str(&format!("<section{id}>\n"));
    }
}

fn emit_paragraph(
    doc: &IrDocument,
    para: &Paragraph,
    out: &mut String,
    opts: &HtmlOptions,
    depth: u8,
    path: &str,
) {
    let body = render_para_text(doc, para, opts);
    let has_text = body_has_visible(&body);
    // Headings nest one level less than their own depth so their
    // body content lines up a step deeper than the heading itself.
    // Non-heading content uses `depth` directly.
    let heading_indent = depth.saturating_sub(1);
    let body_indent = depth;
    let para_id = format!(r#" id="par-{path}""#);

    if has_text {
        match heading_level(doc, para) {
            Some(level) => {
                let tag = format!("h{}", level.clamp(1, 6));
                out.push('<');
                out.push_str(&tag);
                out.push_str(&para_id);
                out.push_str(&paragraph_style_attr(doc, para, heading_indent));
                out.push('>');
                out.push_str(&body);
                out.push_str("</");
                out.push_str(&tag);
                out.push_str(">\n");
            }
            None => {
                out.push_str("<p");
                out.push_str(&para_id);
                out.push_str(&paragraph_style_attr(doc, para, body_indent));
                out.push('>');
                out.push_str(&body);
                out.push_str("</p>\n");
            }
        }
    }
    for (ci, c) in para.controls.iter().enumerate() {
        let ctrl_path = format!("{path}-c{ci}");
        // Tables and figures follow the body indent so they line up
        // with the paragraph text under the current heading.
        let indent = body_indent;
        match &c.kind {
            ControlKind::Table(t) => {
                if indent > 0 {
                    out.push_str(&format!(
                        r#"<div class="indent"{}>"#,
                        indent_style_attr(indent),
                    ));
                }
                emit_table(doc, t, out, opts, &ctrl_path);
                if indent > 0 {
                    out.push_str("</div>\n");
                }
            }
            ControlKind::Picture(p) => {
                if indent > 0 {
                    out.push_str(&format!(
                        r#"<div class="indent"{}>"#,
                        indent_style_attr(indent),
                    ));
                }
                emit_figure(doc, p, c.caption_text.as_deref(), out, opts, &ctrl_path);
                if indent > 0 {
                    out.push_str("</div>\n");
                }
            }
            _ => {}
        }
    }
}

/// Build the ` style="padding-left:Nem"` attribute (with leading
/// space) for the given depth. Kept in one place so the unit and
/// spacing decision stay consistent.
fn indent_style_attr(depth: u8) -> String {
    // 1em per heading level. 2 spaces ≈ 1em in Noto Sans KR body
    // copy, matching the "스페이스 2개 정도" ask in practice.
    format!(r#" style="padding-left:{depth}em""#)
}

/// Build a combined paragraph `style="…"` declaration from the
/// heading-depth indent and the paragraph's ParaShape-declared
/// alignment. Returns an empty string when nothing applies so the
/// common case (left-aligned body text at depth 0) doesn't litter
/// the DOM with empty attributes.
///
/// CSS `text-align` values:
///
///   * 0 / 4 / 5 (JUSTIFY / DISTRIBUTE / DISTRIBUTE_SPACE) →
///     `text-align:justify`.
///   * 1 (LEFT) → skip (default).
///   * 2 (RIGHT) → `text-align:right`.
///   * 3 (CENTER) → `text-align:center`.
///
/// When both indent and alignment apply they're joined with `;`
/// into a single `style="…"` attribute.
fn paragraph_style_attr(doc: &IrDocument, para: &Paragraph, indent: u8) -> String {
    let mut parts: Vec<String> = Vec::new();
    if indent > 0 {
        parts.push(format!("padding-left:{indent}em"));
    }
    if let Some(css) = paragraph_text_align(doc, para) {
        parts.push(format!("text-align:{css}"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(r#" style="{}""#, parts.join(";"))
    }
}

/// Look up the paragraph's `ParaShape.align()` via its
/// `para_shape_id` and translate to the CSS `text-align` keyword.
/// Returns `None` when the shape is missing, out of range, or
/// aligns left (browser default — emitting it would just bulk up
/// the HTML for no visual change).
fn paragraph_text_align(doc: &IrDocument, para: &Paragraph) -> Option<&'static str> {
    let shape = doc
        .doc_info
        .para_shapes
        .get(para.header.para_shape_id as usize)?;
    match shape.align() {
        // Left is the default; skip to keep the HTML tidy.
        1 => None,
        0 | 4 | 5 => Some("justify"),
        2 => Some("right"),
        3 => Some("center"),
        _ => None,
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

fn emit_table(
    doc: &IrDocument,
    t: &TableControl,
    out: &mut String,
    opts: &HtmlOptions,
    path: &str,
) {
    out.push_str(&format!(r#"<table id="tbl-{path}">"#));
    out.push('\n');
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
            let cell_path = format!("{path}-r{}c{}", cell.row, cell.col);
            emit_cell(doc, cell, out, opts, &cell_path);
        }
        out.push_str("  </tr>\n");
    }
    out.push_str("</table>\n");
}

fn emit_cell(
    doc: &IrDocument,
    cell: &TableCell,
    out: &mut String,
    opts: &HtmlOptions,
    path: &str,
) {
    out.push_str(&format!(r#"    <td id="cell-{path}""#));
    if cell.row_span > 1 {
        out.push_str(&format!(" rowspan=\"{}\"", cell.row_span));
    }
    if cell.col_span > 1 {
        out.push_str(&format!(" colspan=\"{}\"", cell.col_span));
    }
    out.push_str(">");
    let mut wrote_text = false;
    for (pi, p) in cell.paragraphs.iter().enumerate() {
        let body = render_para_text(doc, p, opts);
        if body_has_visible(&body) {
            if wrote_text {
                out.push_str("<br>");
            }
            out.push_str(&body);
            wrote_text = true;
        }
        let para_path = format!("{path}-p{pi}");
        for (ci, ctrl) in p.controls.iter().enumerate() {
            let nested_path = format!("{para_path}-c{ci}");
            match &ctrl.kind {
                ControlKind::Table(nested) => {
                    emit_table(doc, nested, out, opts, &nested_path)
                }
                ControlKind::Picture(pic) => emit_figure(
                    doc,
                    pic,
                    ctrl.caption_text.as_deref(),
                    out,
                    opts,
                    &nested_path,
                ),
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
    path: &str,
) {
    // Centred block-level figure so images anchor in the column
    // rather than left-hugging the text; the HWP picture-control
    // position metadata (`horzAlign` / `vertRelTo` / offsets) isn't
    // in the typed IR yet, so "centred" is the safer default for
    // floating images and mirrors what Hancom viewers show when no
    // explicit alignment is set. Figure id follows the bin_id so
    // anchors stay stable even if the paragraph ordering changes;
    // `path` is carried for compat with the rest of the tree.
    let _ = path;
    let fig_id = format!("fig-{}", pic.bin_id);
    out.push_str(&format!(
        r#"<figure id="{fig_id}" style="margin:0.6em auto;text-align:center">"#
    ));
    out.push('\n');
    let w_mm = hwpunit_to_mm(pic.width_hwpu);
    let h_mm = hwpunit_to_mm(pic.height_hwpu);
    let src = if let Some(prefix) = &opts.assets_path {
        // Explicit assets dir wins — caller has its own sidecar setup.
        // Prefer the actual `BinaryEntry.id` when we have one, so both
        // HWP5 (`BIN000A.png`) and HWPX (`image1.png`) conventions
        // resolve. Fall back to HWP5's conventional hex-formatted
        // filename when `doc.bin_data` is empty — covers docs whose
        // raw bytes were stripped but whose DocInfo BinData list
        // still identifies the extension.
        let filename = doc
            .bin_data
            .iter()
            .find(|e| matches_bin_id(&e.id, pic.bin_id))
            .map(|entry| entry.id.clone())
            .unwrap_or_else(|| {
                format!(
                    "BIN{:04X}.{}",
                    pic.bin_id,
                    resolve_bin_extension(doc, pic.bin_id),
                )
            });
        Some(format!("{}/{}", escape_attr(prefix), escape_attr(&filename)))
    } else {
        // No assets dir — inline as a data URI so the HTML is self-
        // contained (preview in an iframe without blob-URL plumbing,
        // PDF print with embedded images, single-file copy). Falls
        // through to caption-only when the binary isn't resident.
        resolve_bin_data_uri(doc, pic.bin_id)
    };

    if let Some(src) = src {
        // The HWP picture control's width / height carry the
        // document's declared display size. Using `aspect-ratio`
        // from those HWPUNIT values (ratios stay identical after
        // mm rounding) preserves HWP-authored scaling — even
        // deliberately stretched or squished images keep their
        // intended proportions. `max-width:100%` still lets the
        // preview pane scale the figure down when the pane is
        // narrower than the declared width.
        let aspect = if pic.width_hwpu > 0 && pic.height_hwpu > 0 {
            format!(";aspect-ratio:{}/{}", pic.width_hwpu, pic.height_hwpu)
        } else {
            // Missing dimensions (old HWPX docs or shapes we haven't
            // fully parsed) — fall back to the image's intrinsic
            // aspect ratio via `height:auto`.
            ";height:auto".to_string()
        };
        out.push_str(&format!(
            r#"  <img src="{}" style="display:block;margin:0 auto;width:{}mm;max-width:100%{}">{}"#,
            src, w_mm, aspect, "\n",
        ));
    }

    if let Some(cap) = caption_text {
        let cleaned = clean_text(cap);
        let stripped = strip_caption_label_prefix(&cleaned).trim();
        if !stripped.is_empty() {
            out.push_str(&format!(
                r#"  <figcaption id="cap-{fig_id}" style="margin-top:0.25em;font-size:0.9em;color:#555">"#
            ));
            out.push_str(&escape_html(stripped));
            out.push_str("</figcaption>\n");
        }
    }
    out.push_str("</figure>\n");
}

fn resolve_bin_extension(doc: &IrDocument, bin_id: u16) -> String {
    // HWP5 path: DocInfo's typed BinData list carries the extension
    // per-record. When present, prefer that.
    if let Some(ext) = doc
        .doc_info
        .bin_data
        .iter()
        .find(|bd: &&BinData| bd.bin_data_id == Some(bin_id))
        .and_then(|bd| bd.extension.as_deref())
    {
        return ext.to_string();
    }
    // HWPX path: the archive's `BinData/image{N}.{ext}` is parked
    // directly in `doc.bin_data` — pull the extension from the
    // filename. Matches either HWPX pattern (`image1.png`) or a raw
    // HWP5 leaf that slipped through (`BIN0001.png`).
    for entry in &doc.bin_data {
        if matches_bin_id(&entry.id, bin_id) {
            if let Some(ext) = entry.id.rsplit_once('.').map(|(_, e)| e.to_string()) {
                return ext;
            }
        }
    }
    "bin".to_string()
}

/// True when the `BinaryEntry.id` filename matches the numeric
/// bin_id under either container's naming convention.
fn matches_bin_id(id: &str, bin_id: u16) -> bool {
    // HWPX: `image{dec}.{ext}`
    if let Some(stem) = id.strip_prefix("image") {
        if let Some((num, _)) = stem.split_once('.') {
            if num.parse::<u16>() == Ok(bin_id) {
                return true;
            }
        }
    }
    // HWP5: `BIN{HEX}.{ext}` — zero-padded 4 hex digits.
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

/// Build a `data:<mime>;base64,<payload>` URI for the picture's
/// embedded binary. Returns `None` when the binary isn't resident in
/// `doc.bin_data` (e.g. the file was opened with images stripped) or
/// can't be decoded into a browser-compatible format.
///
/// Formats that browsers render directly (PNG / JPEG / GIF / WebP /
/// SVG) pass through unchanged. Legacy HWP-native formats that
/// browsers handle inconsistently (BMP, TIFF, DDS) get transcoded to
/// JPEG via the `image` crate before embedding — better a slightly
/// larger JPEG than a "broken image" icon in the preview. Anything
/// the decoder can't recognise (WMF / EMF for now) returns `None`
/// so the emitter drops the `<img>` and falls through to the
/// caption-only figure.
fn resolve_bin_data_uri(doc: &IrDocument, bin_id: u16) -> Option<String> {
    let entry = doc.bin_data.iter().find(|e| matches_bin_id(&e.id, bin_id))?;
    if entry.bytes.is_empty() {
        return None;
    }
    let ext = entry
        .id
        .rsplit_once('.')
        .map(|(_, e)| e.to_string())
        .unwrap_or_else(|| "bin".to_string());
    if is_web_native(&ext) {
        let mime = entry.mime.clone().unwrap_or_else(|| mime_for_ext(&ext));
        let payload = base64_encode(&entry.bytes);
        return Some(format!("data:{mime};base64,{payload}"));
    }
    // Legacy path: try to transcode to JPEG. JPEG wins over PNG here
    // because HWP BMPs are usually either photos or scanned figures
    // — both re-compress cleanly; PNG's lossless cost isn't worth it
    // for preview. Quality 90 keeps size reasonable while staying
    // well clear of visible blocking.
    transcode_to_jpeg(&entry.bytes).map(|(mime, bytes)| {
        format!("data:{mime};base64,{}", base64_encode(&bytes))
    })
}

/// Formats the browser renders directly via `<img src="data:…">`.
/// The exhaustive list keeps us from quietly sending a container
/// format (AVIF / HEIC / ...) we haven't learned yet.
fn is_web_native(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg"
    )
}

/// Decode a legacy image via the `image` crate and re-encode it as
/// JPEG. Returns `None` when `image` can't recognise the container
/// (WMF / EMF / proprietary) — caller drops the `<img>` tag in that
/// case rather than embed unreadable bytes.
fn transcode_to_jpeg(bytes: &[u8]) -> Option<(String, Vec<u8>)> {
    let img = image::load_from_memory(bytes).ok()?;
    // JPEG doesn't support alpha, so collapse RGBA surfaces onto
    // white before encoding to avoid the image crate erroring or
    // producing transparent-artefact JPEGs.
    let rgb = img.to_rgb8();
    let mut buf: Vec<u8> = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut buf);
    let encoder =
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, 90);
    image::ImageEncoder::write_image(
        encoder,
        rgb.as_raw(),
        rgb.width(),
        rgb.height(),
        image::ExtendedColorType::Rgb8,
    )
    .ok()?;
    Some(("image/jpeg".to_string(), buf))
}

fn mime_for_ext(ext: &str) -> String {
    match ext.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "tif" | "tiff" => "image/tiff",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Standard base64 (RFC 4648) — no line breaks. Avoids pulling in
/// the `base64` crate for one call site; HWP image blobs are already
/// compressed so a single pass is fine.
fn base64_encode(data: &[u8]) -> String {
    const ALPHA: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= data.len() {
        let triple = ((data[i] as u32) << 16)
            | ((data[i + 1] as u32) << 8)
            | (data[i + 2] as u32);
        out.push(ALPHA[((triple >> 18) & 0x3F) as usize] as char);
        out.push(ALPHA[((triple >> 12) & 0x3F) as usize] as char);
        out.push(ALPHA[((triple >> 6) & 0x3F) as usize] as char);
        out.push(ALPHA[(triple & 0x3F) as usize] as char);
        i += 3;
    }
    let rem = data.len() - i;
    match rem {
        1 => {
            let triple = (data[i] as u32) << 16;
            out.push(ALPHA[((triple >> 18) & 0x3F) as usize] as char);
            out.push(ALPHA[((triple >> 12) & 0x3F) as usize] as char);
            out.push_str("==");
        }
        2 => {
            let triple =
                ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8);
            out.push(ALPHA[((triple >> 18) & 0x3F) as usize] as char);
            out.push(ALPHA[((triple >> 12) & 0x3F) as usize] as char);
            out.push(ALPHA[((triple >> 6) & 0x3F) as usize] as char);
            out.push('=');
        }
        _ => {}
    }
    out
}

fn hwpunit_to_mm(hwpu: u32) -> u32 {
    ((hwpu as f64) * 25.4 / 7200.0).round() as u32
}

/// Try to classify a paragraph as a heading via two independent
/// signals so that subsection indentation reflects both documents
/// that use Hancom's "개요" style and documents that only use plain
/// numeric chapter prefixes.
///
///   1. Style-based — `Style.name` begins with one of the known
///      outline tokens (`"개요 N"`, `"Heading N"`, `"제목 N"`,
///      `"Outline N"`), or matches the table-of-contents title.
///   2. Text-prefix fallback — the paragraph's first non-whitespace
///      bytes look like a chapter number (`"1."`, `"1.1"`,
///      `"1.1.2."`), the text is short enough to plausibly be a
///      heading, and it isn't punctuation-terminated prose.
///
/// Only one signal needs to fire. Returning None leaves the depth
/// tracker at the previous heading's level — which is the safer
/// default for ambiguous paragraphs.
fn heading_level(doc: &IrDocument, para: &Paragraph) -> Option<u8> {
    if let Some(style) = doc.doc_info.styles.get(para.header.style_id as usize) {
        for prefix in ["개요 ", "Outline ", "Heading ", "제목 "] {
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
    }
    heading_level_from_text_prefix(&para.text)
}

/// Numeric-prefix heading detector. Handles:
///
///   "1. 제목"       → level 1
///   "1.1 제목"      → level 2
///   "1.1. 제목"     → level 2 (trailing dot optional)
///   "1.1.2. 제목"   → level 3
///
/// Guardrails: text must be short enough to be a plausible heading
/// (≤ 100 chars), must have non-empty numeric prefix and following
/// whitespace + text, and must not end with a Korean sentence
/// terminator (`"다."` / `"함."` / `"임."`) — those signal prose,
/// not a heading.
fn heading_level_from_text_prefix(raw: &str) -> Option<u8> {
    // Use the cleaned text so PUA bullets / FFFC don't break prefix
    // matching. Length budget is chars, not bytes — Korean headings
    // hit the byte ceiling quickly otherwise.
    let text = clean_text(raw);
    let trimmed = text.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().count() > 100 {
        return None;
    }
    // Peel leading digits and dots off the prefix.
    let prefix: String = trimmed
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    if prefix.is_empty() || !prefix.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let rest = &trimmed[prefix.len()..];
    // Must be followed by whitespace — rules out "1.2.3만" (Korean
    // postposition stuck to a number).
    let Some(first) = rest.chars().next() else {
        return None;
    };
    if !first.is_whitespace() {
        return None;
    }
    // Likely sentence-as-prose guard: ends with 다., 함., 임., 음.
    // — Korean declarative endings. A real heading almost never
    // closes with one of those.
    let tail: String = trimmed.chars().rev().take(3).collect::<String>()
        .chars().rev().collect();
    for ender in ["다.", "함.", "임.", "음."] {
        if tail.ends_with(ender) {
            return None;
        }
    }
    let parts: Vec<&str> = prefix
        .split('.')
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }
    if !parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    Some((parts.len() as u8).min(6))
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
        assert!(html.contains("안녕하세요</p>"), "got: {html}");
        assert!(html.contains(r#"<p id="par-s0-p0""#), "got: {html}");
    }

    #[test]
    fn outline_style_becomes_heading() {
        let doc = make_doc(
            vec![style("본문"), style("개요 2")],
            vec![para(1, "2장 제목")],
        );
        let html = to_html(&doc);
        // H2 sits one indent level under H1 so subsection content
        // visually nests underneath. Open tag carries the inline
        // padding-left style; closing tag and body are unchanged.
        assert!(
            html.contains(r#"style="padding-left:1em">2장 제목</h2>"#),
            "got: {html}"
        );
    }

    #[test]
    fn subsection_content_indents_under_heading() {
        // Heading at level 2 → following body paragraph indents to
        // depth=2 (`padding-left:2em`) so it reads as "subsection
        // content" rather than a sibling of the heading.
        let doc = make_doc(
            vec![style("본문"), style("개요 2")],
            vec![para(1, "2장 제목"), para(0, "본문 한 줄")],
        );
        let html = to_html(&doc);
        assert!(
            html.contains(r#"style="padding-left:2em">본문 한 줄</p>"#),
            "subsection content should indent: {html}"
        );
    }

    #[test]
    fn h1_at_top_level_stays_flush_left() {
        // Level 1 heading: heading_indent=0, body_indent=1. The
        // heading itself shouldn't carry padding-left.
        let doc = make_doc(
            vec![style("본문"), style("개요 1")],
            vec![para(1, "1장 제목"), para(0, "첫 줄")],
        );
        let html = to_html(&doc);
        assert!(html.contains("1장 제목</h1>"), "got: {html}");
        assert!(
            html.contains(r#"style="padding-left:1em">첫 줄</p>"#),
            "got: {html}"
        );
    }

    #[test]
    fn numeric_prefix_detected_as_heading() {
        // Doc without "개요 N" styles — relies on the text-prefix
        // heuristic so subsection indent still applies.
        let doc = make_doc(
            vec![style("본문")],
            vec![
                para(0, "1. 사업 개요"),
                para(0, "본문 첫 줄"),
                para(0, "1.1 세부 과제"),
                para(0, "본문 둘째 줄"),
                para(0, "1.1.1 더 세부"),
                para(0, "본문 셋째 줄"),
            ],
        );
        let html = to_html(&doc);
        // H1 heading from "1." prefix, no indent on itself.
        assert!(html.contains("1. 사업 개요</h1>"), "got: {html}");
        // Body below H1 indents one level.
        assert!(
            html.contains(r#"style="padding-left:1em">본문 첫 줄</p>"#),
            "got: {html}"
        );
        // H2 from "1.1" — heading itself indents one, body two.
        assert!(
            html.contains(r#"style="padding-left:1em">1.1 세부 과제</h2>"#),
            "got: {html}"
        );
        assert!(
            html.contains(r#"style="padding-left:2em">본문 둘째 줄</p>"#),
            "got: {html}"
        );
        // H3 from "1.1.1" — heading indents two, body three.
        assert!(
            html.contains(r#"style="padding-left:2em">1.1.1 더 세부</h3>"#),
            "got: {html}"
        );
        assert!(
            html.contains(r#"style="padding-left:3em">본문 셋째 줄</p>"#),
            "got: {html}"
        );
    }

    #[test]
    fn numeric_prefix_in_long_prose_not_promoted() {
        // Too long to be a plausible heading; guard keeps it as <p>.
        let doc = make_doc(
            vec![style("본문")],
            vec![para(0, "1. 본 연구는 여러 가지 복잡한 사항을 다루고 있으며, \
                다양한 관점에서의 분석을 제공함으로써 독자로 하여금 깊이 있는 \
                이해를 가능하게 한다.")],
        );
        let html = to_html(&doc);
        // Should remain a paragraph, not a heading.
        assert!(!html.contains("<h1"), "got: {html}");
        assert!(html.contains("<p "), "got: {html}");
    }

    #[test]
    fn chapter_section_wraps_headings_and_closes_at_same_level() {
        // Sequence: H1, body, H2, body, H1. The second H1 closes
        // both the previous H2 chapter and H1 chapter before opening
        // a new one.
        let doc = make_doc(
            vec![style("본문"), style("개요 1"), style("개요 2")],
            vec![
                para(1, "1장"),
                para(0, "첫 단락"),
                para(2, "1.1절"),
                para(0, "둘째 단락"),
                para(1, "2장"),
                para(0, "셋째 단락"),
            ],
        );
        let html = to_html(&doc);
        // First chapter opens.
        assert!(
            html.contains(r#"<section class="hwp-chapter hwp-lv-1" id="sec-s0-p0">"#),
            "got: {html}"
        );
        // Subchapter opens under first.
        assert!(
            html.contains(r#"<section class="hwp-chapter hwp-lv-2" id="sec-s0-p2">"#),
            "got: {html}"
        );
        // Second H1 closes both prior sections (2 close tags between
        // "둘째 단락" and the new H1's wrapper).
        let idx_close = html
            .find(r#"<section class="hwp-chapter hwp-lv-1" id="sec-s0-p4">"#)
            .expect("second H1 wrapper");
        let before = &html[..idx_close];
        // Count closes just before the second H1: should include at
        // least two `</section>` tags to pop the H2 + first H1.
        let closes_right_before = before
            .rsplit("</section>")
            .take(3)
            .count();
        assert!(
            closes_right_before >= 3,
            "expected two stacked closes before second H1: {html}"
        );
    }

    #[test]
    fn parashape_center_yields_text_align_css() {
        use hwp_transpiler_core::ir::ParaShape;
        let mut doc = IrDocument::default();
        let mut center = ParaShape::default();
        center.attribute = 3; // CENTER
        doc.doc_info.para_shapes.push(center);
        doc.doc_info.styles = vec![style("본문")];
        doc.sections.push(Section {
            paragraphs: vec![Paragraph {
                header: ParagraphHeader {
                    style_id: 0,
                    para_shape_id: 0,
                    ..ParagraphHeader::default()
                },
                text: "가운데 정렬".into(),
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        let html = to_html(&doc);
        assert!(
            html.contains(r#"style="text-align:center""#),
            "got: {html}"
        );
    }

    #[test]
    fn parashape_left_is_default_not_emitted() {
        use hwp_transpiler_core::ir::ParaShape;
        let mut doc = IrDocument::default();
        let mut left = ParaShape::default();
        left.attribute = 1; // LEFT
        doc.doc_info.para_shapes.push(left);
        doc.doc_info.styles = vec![style("본문")];
        doc.sections.push(Section {
            paragraphs: vec![Paragraph {
                header: ParagraphHeader {
                    style_id: 0,
                    para_shape_id: 0,
                    ..ParagraphHeader::default()
                },
                text: "왼쪽 기본".into(),
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        let html = to_html(&doc);
        assert!(
            !html.contains("text-align"),
            "left is default, should not emit: {html}"
        );
    }

    #[test]
    fn parashape_align_combines_with_indent() {
        // Right-aligned heading at depth 2 → both text-align:right
        // and padding-left:1em show up in one style attribute.
        use hwp_transpiler_core::ir::ParaShape;
        let mut doc = IrDocument::default();
        let mut right = ParaShape::default();
        right.attribute = 2; // RIGHT
        doc.doc_info.para_shapes.push(right);
        doc.doc_info.styles = vec![style("본문"), style("개요 2")];
        doc.sections.push(Section {
            paragraphs: vec![Paragraph {
                header: ParagraphHeader {
                    style_id: 1,
                    para_shape_id: 0,
                    ..ParagraphHeader::default()
                },
                text: "오른쪽 제목".into(),
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        let html = to_html(&doc);
        assert!(
            html.contains(r#"style="padding-left:1em;text-align:right""#),
            "got: {html}"
        );
    }

    #[test]
    fn korean_sentence_ending_not_promoted() {
        // Starts with "1." but ends with "다." — clearly prose.
        let doc = make_doc(
            vec![style("본문")],
            vec![para(0, "1. 이는 주요한 의미를 갖는다.")],
        );
        let html = to_html(&doc);
        assert!(!html.contains("<h1"), "got: {html}");
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
        assert!(html.contains("<table id="));
        assert!(html.contains("<tr>"));
        assert!(html.contains("a</td>"));
        assert!(html.contains("d</td>"));
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
            html.contains("colspan=\"3\">merged header</td>"),
            "got: {html}"
        );
        assert!(html.contains("x</td>"));
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
        assert!(html.contains("rowspan=\"2\">vmerge</td>"));
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
        assert!(html.contains("<figure"), "got: {html}");
        // Image source + intrinsic mm dimensions survive alongside
        // the responsive constraints added for preview-pane fit.
        assert!(
            html.contains(r#"src="x.assets/BIN0001.png""#),
            "got: {html}"
        );
        assert!(html.contains("width:25mm"), "got: {html}");
        assert!(html.contains("max-width:100%"), "got: {html}");
        assert!(html.contains("시스템 도식"), "got: {html}");
    }

    #[test]
    fn figure_without_assets_or_bindata_renders_caption_only() {
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
        assert!(html.contains("설명"), "got: {html}");
        assert!(html.contains("<figcaption"), "got: {html}");
    }

    #[test]
    fn figure_inlines_as_data_uri_when_bindata_resident() {
        use hwp_transpiler_core::ir::{BinData, BinaryEntry, PictureControl};
        let mut doc = IrDocument::default();
        doc.doc_info.bin_data.push(BinData {
            bin_data_id: Some(1),
            extension: Some("png".into()),
            ..BinData::default()
        });
        // Fake 3-byte PNG magic to exercise base64 padding-0.
        doc.bin_data.push(BinaryEntry {
            id: "BIN0001.png".into(),
            mime: Some("image/png".into()),
            bytes: vec![0x89, b'P', b'N'],
        });
        doc.sections.push(Section {
            paragraphs: vec![Paragraph {
                controls: vec![Control {
                    kind: ControlKind::Picture(PictureControl {
                        bin_id: 1,
                        width_hwpu: 7200,
                        height_hwpu: 3600,
                    }),
                    caption_text: None,
                }],
                ..Paragraph::default()
            }],
            ..Section::default()
        });
        let html = to_html(&doc);
        assert!(
            html.contains("src=\"data:image/png;base64,"),
            "got: {html}"
        );
        // 3 bytes → 4 base64 chars, no trailing `=`.
        assert!(html.contains("iVBO"), "base64 of 0x89 'P' 'N': {html}");
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
        assert!(html.contains("ABC</p>"), "got: {html}");
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
        assert!(html.contains("<section id=\"sec-0\">"), "got: {html}");
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
            html.contains("class=\"hwp-page\""),
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
        assert!(html.contains("class=\"hwp-page\">"), "got: {html}");
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
