//! LLM-mode Markdown → [`IrDocument`] importer.
//!
//! Inverse of `export::markdown_llm`. The LLM emit path produces a
//! line-record format that's still UTF-8 Markdown but record-shaped
//! rather than prose-shaped:
//!
//! ```text
//! SECTION[id=sec-0]
//!
//! PARAGRAPH[id=par-s0-p0]
//! TEXT: 본문 단락 텍스트.
//!
//! TABLE[id=tbl-s0-p1-c0,rows=3,cols=2,kind=schedule]
//! CELL[id=cell-…,row=0,col=0,rowspan=1,colspan=1,role=header]
//! TEXT[par-…-r0c0-p0]: 항목
//! …
//! END TABLE[tbl-s0-p1-c0]
//! ```
//!
//! GFM tables go through `import::markdown::from_markdown`; this path
//! handles the alternative LLM emit. `from_llm_markdown` is the
//! direct entrypoint; `markdown::from_markdown` auto-detects via the
//! leading `SECTION[id=` sigil and routes here.
//!
//! Phase-1 scope of this slice: `SECTION`, `PARAGRAPH`, `TABLE`,
//! `CELL`, `TEXT`, and `END TABLE` records. Figure / caption /
//! equation records aren't yet decoded — they fall through as
//! ignored lines, which is graceful but lossy.

use std::collections::HashMap;

use hwp_transpiler_core::ir::{
    CharShapeRun, Control, ControlKind, IrDocument, IrError, LineSegment, Paragraph,
    ParagraphHeader, PictureControl, Section, TableCell, TableControl,
};

use crate::asset_pipeline::decode_data_uri_to_binary_entry;

use base64::{engine::general_purpose::STANDARD, Engine as _};

/// Decode the `data:application/octet-stream;base64,…` URI emitted
/// alongside `SECTION_BYTES` records. Returns `None` for any non-
/// base64 / non-data scheme — the caller drops the section bytes
/// silently in that case rather than corrupting `stream_bytes`.
fn decode_octet_stream_data_uri(uri: &str) -> Option<Vec<u8>> {
    let rest = uri.strip_prefix("data:")?;
    let (header, payload) = rest.split_once(',')?;
    let (_mime, encoding) = header
        .split_once(';')
        .map(|(m, e)| (m.trim(), e.trim()))
        .unwrap_or((header.trim(), ""));
    if encoding != "base64" {
        return None;
    }
    STANDARD.decode(payload).ok()
}

/// Returns true when `src` is recognisably the LLM-mode export
/// format. Uses the sigil our exporter emits at the top of every
/// document — `SECTION[id=sec-…]` as the first non-blank line.
pub fn looks_like_llm_format(src: &str) -> bool {
    src.lines()
        .map(|l| l.trim_start())
        .find(|l| !l.is_empty())
        .map(|l| l.starts_with("SECTION[id="))
        .unwrap_or(false)
}

/// Parse the LLM-mode export back into an `IrDocument`. Always
/// produces at least one section (matching the empty-input behaviour
/// of `from_markdown`).
pub fn from_llm_markdown(src: &str) -> Result<IrDocument, IrError> {
    let mut doc = IrDocument::default();
    // Same 7-slot heading/styles/charShape table the GFM importer
    // builds, so heading round-trip is consistent across both MD
    // import paths and the HWPX writer's surgical rewriter has the
    // shapes to splice into the bundled skeleton.
    super::style_synth::populate_heading_doc_info(&mut doc);

    let mut sections: Vec<Section> = Vec::new();
    let mut current = Section::default();
    let mut state = State::Idle;
    // True once we've crossed `<!-- hwp-transpiler: assets -->`. From
    // that point on, body-level records (FIGURE / TABLE / PARAGRAPH)
    // stop landing into the section and ASSET / DATA pairs flow into
    // `doc.bin_data` instead.
    let mut in_assets = false;
    let mut pending_asset: Option<PendingAsset> = None;
    // When a `SECTION_BYTES[id=section-N,…]` record is seen, the
    // following `DATA:` line decodes into `sections[N].stream_bytes`.
    // Mutually exclusive with `pending_asset` — both branches read
    // the next DATA line from the same handler.
    let mut pending_section_idx: Option<usize> = None;
    // Same handoff for `UNKNOWN_STREAM[name=…,…]` — the bytes that
    // follow restore `doc.unknown_streams[name]` so non-section
    // ZIP entries (META-INF/, settings.xml, version.xml, Preview/)
    // round-trip verbatim.
    let mut pending_stream_name: Option<String> = None;
    // `DOC_INFO[len=N]` followed by a JSON `DATA:` line restores the
    // entire `doc.doc_info` (font_faces / border_fills / char_shapes /
    // para_shapes / styles / properties). HWP5 sources don't carry a
    // `Contents/header.xml` for the post-loop reparse to feed off, so
    // without this record every paragraph collapses to default
    // `paraPrIDRef="0"` after round-trip.
    let mut pending_doc_info = false;

    for line in src.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed == "<!-- hwp-transpiler: assets -->" {
            // Flush any in-flight body state and switch to footer
            // mode. Subsequent SECTION[ markers (defensive) are
            // ignored — assets aren't sectioned.
            flush_state(&mut state, &mut current);
            sections.push(std::mem::take(&mut current));
            state = State::Idle;
            in_assets = true;
            continue;
        }

        if in_assets {
            if trimmed.starts_with("ASSET[") {
                let attrs = parse_attrs(trimmed);
                let bin_id = attrs.get_int("bin_id").map(|n| n.clamp(0, u16::MAX as i64) as u16);
                let source_id = attrs
                    .0
                    .get("source_id")
                    .cloned()
                    .or_else(|| {
                        // Fall back to a sensible default name when the exporter
                        // didn't stamp one. HWPX convention.
                        bin_id.map(|n| format!("image{n}.png"))
                    })
                    .unwrap_or_default();
                pending_asset = Some(PendingAsset { bin_id, source_id });
                pending_section_idx = None;
                pending_doc_info = false;
                continue;
            }
            if trimmed.starts_with("SECTION_BYTES[") {
                let attrs = parse_attrs(trimmed);
                // id="section-N" → N
                pending_section_idx = attrs
                    .0
                    .get("id")
                    .and_then(|s| s.strip_prefix("section-"))
                    .and_then(|s| s.parse::<usize>().ok());
                pending_asset = None;
                pending_stream_name = None;
                pending_doc_info = false;
                continue;
            }
            if trimmed.starts_with("UNKNOWN_STREAM[") {
                let attrs = parse_attrs(trimmed);
                pending_stream_name = attrs.0.get("name").cloned();
                pending_asset = None;
                pending_section_idx = None;
                pending_doc_info = false;
                continue;
            }
            if trimmed.starts_with("DOC_INFO[") {
                pending_doc_info = true;
                pending_asset = None;
                pending_section_idx = None;
                pending_stream_name = None;
                continue;
            }
            if let Some(uri) = trimmed.strip_prefix("DATA: ") {
                if let Some(p) = pending_asset.take() {
                    if let Some(entry) = decode_data_uri_to_binary_entry(uri.trim(), &p.source_id) {
                        doc.bin_data.push(entry);
                    }
                } else if let Some(idx) = pending_section_idx.take() {
                    if let Some(bytes) = decode_octet_stream_data_uri(uri.trim()) {
                        if let Some(section) = sections.get_mut(idx) {
                            section.stream_bytes = Some(bytes);
                        }
                    }
                } else if let Some(name) = pending_stream_name.take() {
                    if let Some(bytes) = decode_octet_stream_data_uri(uri.trim()) {
                        doc.unknown_streams.insert(name, bytes);
                    }
                } else if std::mem::take(&mut pending_doc_info) {
                    if let Some(bytes) = decode_octet_stream_data_uri(uri.trim()) {
                        if let Ok(info) = serde_json::from_slice::<
                            hwp_transpiler_core::ir::DocInfo,
                        >(&bytes)
                        {
                            doc.doc_info = info;
                        }
                    }
                }
                continue;
            }
            // Other lines in footer (blank, comments) — ignore.
            continue;
        }

        if trimmed.starts_with("SECTION[") {
            // Flush prior pending state into the current section,
            // then push it (but only if it actually carries
            // anything — the first SECTION marker arrives before
            // any content, so an empty `current` here is the
            // leading buffer and would shift `SECTION_BYTES
            // [id=section-N]` indexing off by one).
            flush_state(&mut state, &mut current);
            if !current.paragraphs.is_empty() || current.stream_bytes.is_some() {
                sections.push(std::mem::take(&mut current));
            } else {
                current = Section::default();
            }
            // `sec_pr=<base64>` restores the source `<hp:secPr>`
            // verbatim so rebuilt sections keep the original page
            // geometry (margins decide body width → line wrapping).
            if let Some(b64) = parse_attrs(trimmed).0.get("sec_pr") {
                if let Some(xml) = STANDARD
                    .decode(b64)
                    .ok()
                    .and_then(|b| String::from_utf8(b).ok())
                {
                    current.sec_pr_xml = Some(xml);
                }
            }
            state = State::Idle;
            continue;
        }

        if trimmed.starts_with("PARAGRAPH[") {
            flush_state(&mut state, &mut current);
            // Heading level can be supplied two ways:
            //   * Explicit `level=N` attribute on the PARAGRAPH
            //     record — preferred, requires exporter cooperation.
            //   * Markdown-style `# `…`###### ` text prefix on the
            //     following TEXT line — fallback for already-emitted
            //     docs that don't carry the attr.
            let attrs = parse_attrs(trimmed);
            let level = attrs.get_int("level").map(|n| n.clamp(0, 6) as u8);
            // `para_shape` / `char_shape` route the paragraph back to
            // the right slot in `doc_info.{para,char}_shapes`. Without
            // these every paragraph collapsed to slot 0 on round-trip
            // and HWP5-sourced layout was uniform across the doc.
            let para_shape = attrs
                .get_int("para_shape")
                .map(|n| n.clamp(0, u32::MAX as i64) as u32)
                .unwrap_or(0);
            let char_shape = attrs
                .get_int("char_shape")
                .map(|n| n.clamp(0, u32::MAX as i64) as u32)
                .unwrap_or(0);
            let line_segments = attrs
                .0
                .get("lineseg")
                .map(|v| crate::lineseg_codec::decode(v))
                .unwrap_or_default();
            state = State::ExpectingParagraphText {
                explicit_level: level,
                para_shape_id: para_shape,
                char_shape_id: char_shape,
                line_segments,
                page_break: attrs.get_int("page_break").unwrap_or(0) != 0,
            };
            continue;
        }

        if trimmed.starts_with("TABLE[") {
            let attrs = parse_attrs(trimmed);
            let mut builder = LlmTableBuilder::default();
            builder.border_fill_id = attrs.get_int("border_fill").unwrap_or(0).max(0) as u16;
            if let Some(margins) = attrs.0.get("in_margin") {
                let vals: Vec<i16> = margins
                    .split(':')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
                if vals.len() == 4 {
                    builder.padding = [vals[0], vals[1], vals[2], vals[3]];
                }
            }
            builder.cell_spacing = attrs.get_int("cell_spacing").unwrap_or(0) as i16;
            if let State::InTable(stack) = &mut state {
                // Nested table — the exporter emits these inside the
                // owning CELL block. Push; END TABLE pops and attaches.
                stack.push(builder);
            } else {
                // A PARAGRAPH record immediately before TABLE is the
                // table's anchor: its shape ids describe the wrapper
                // paragraph the table lives in, not a blank line —
                // consume it instead of materialising an empty
                // paragraph via flush.
                if let State::ExpectingParagraphText {
                    para_shape_id,
                    char_shape_id,
                    page_break,
                    ..
                } = std::mem::replace(&mut state, State::Idle)
                {
                    builder.anchor = Some((para_shape_id, char_shape_id, page_break));
                }
                flush_state(&mut state, &mut current);
                state = State::InTable(vec![builder]);
            }
            continue;
        }

        if trimmed.starts_with("END TABLE") {
            if let State::InTable(stack) = &mut state {
                if let Some(builder) = stack.pop() {
                    let anchor = builder.anchor;
                    let table = builder.finish();
                    let wrapper = table_wrapper(table, anchor);
                    if let Some(parent) = stack.last_mut() {
                        parent.attach_paragraph(wrapper);
                    } else {
                        current.paragraphs.push(wrapper);
                        state = State::Idle;
                    }
                }
            }
            continue;
        }

        if trimmed.starts_with("FIGURE[") {
            // A preceding PARAGRAPH record is this figure's anchor —
            // consume its shape ids for the wrapper instead of
            // materialising an empty paragraph. A figure inside a
            // CELL block must not terminate the table being built.
            let mut fig_anchor: Option<(u32, u32, bool)> = None;
            if matches!(state, State::ExpectingParagraphText { .. }) {
                if let State::ExpectingParagraphText {
                    para_shape_id,
                    char_shape_id,
                    page_break,
                    ..
                } = std::mem::replace(&mut state, State::Idle)
                {
                    fig_anchor = Some((para_shape_id, char_shape_id, page_break));
                }
            }
            if !matches!(state, State::InTable(_)) {
                flush_state(&mut state, &mut current);
            }
            let attrs = parse_attrs(trimmed);
            let bin_id = attrs.get_int("bin_id").map(|n| n.clamp(0, u16::MAX as i64) as u16);
            let width_mm = attrs.get_int("width_mm").unwrap_or(0).max(0) as u32;
            let height_mm = attrs.get_int("height_mm").unwrap_or(0).max(0) as u32;
            // mm → HWPUNIT (7200 / 25.4 = ~283.46). Multiply via
            // f64 then round to keep the conversion symmetric with
            // the export side's `hwpunit_to_mm`.
            let width_hwpu = ((width_mm as f64) * 7200.0 / 25.4).round() as u32;
            let height_hwpu = ((height_mm as f64) * 7200.0 / 25.4).round() as u32;
            let mut wrapper = Paragraph::default();
            wrapper.text = "\u{FFFC}".into();
            wrapper.controls.push(Control {
                kind: ControlKind::Picture(PictureControl {
                    bin_id: bin_id.unwrap_or(0),
                    width_hwpu,
                    height_hwpu,
                }),
                caption_text: None,
            });
            if let Some((ps, cs, pb)) = fig_anchor {
                wrapper.header.para_shape_id = ps as u16;
                wrapper.header.page_break_before = pb;
                if cs != 0 {
                    wrapper.char_shape_runs.push(CharShapeRun {
                        start: 0,
                        char_shape_id: cs,
                    });
                }
            }
            if let State::InTable(stack) = &mut state {
                if let Some(builder) = stack.last_mut() {
                    builder.attach_paragraph(wrapper);
                }
            } else {
                current.paragraphs.push(wrapper);
            }
            continue;
        }

        if trimmed.starts_with("CELL[") {
            let attrs = parse_attrs(trimmed);
            let row = attrs.get_int("row").unwrap_or(0) as u16;
            let col = attrs.get_int("col").unwrap_or(0) as u16;
            let row_span = attrs.get_int("rowspan").unwrap_or(1).max(1) as u16;
            let col_span = attrs.get_int("colspan").unwrap_or(1).max(1) as u16;
            let border_fill_id = attrs.get_int("border_fill").unwrap_or(1).max(0) as u16;
            let width_hwpu = attrs.get_int("width").unwrap_or(0).max(0) as u32;
            let height_hwpu = attrs.get_int("height").unwrap_or(0).max(0) as u32;
            let text_width_hwpu = attrs.get_int("text_width").unwrap_or(0).max(0) as u32;
            if let State::InTable(stack) = &mut state {
                if let Some(builder) = stack.last_mut() {
                    builder.flush_pending();
                    builder.pending = Some(PendingCell {
                        row, col, row_span, col_span, border_fill_id,
                        width_hwpu, height_hwpu, text_width_hwpu,
                    });
                }
            }
            continue;
        }

        if let Some((text, line_ps, line_cs, line_segs)) = parse_text_line(trimmed) {
            let text = unescape_text(text);
            match &mut state {
                State::ExpectingParagraphText {
                    explicit_level,
                    para_shape_id,
                    char_shape_id,
                    line_segments,
                    page_break,
                } => {
                    let (level, body) = resolve_heading(*explicit_level, &text);
                    let ps = *para_shape_id;
                    let cs = *char_shape_id;
                    // Top-level paragraphs carry layout on the PARAGRAPH
                    // record (the TEXT line is bare), so prefer the
                    // state's segments and fall back to any on the line.
                    let segs = if line_segments.is_empty() {
                        line_segs
                    } else {
                        std::mem::take(line_segments)
                    };
                    let mut p = make_paragraph(level, body, ps, cs, segs);
                    p.header.page_break_before = *page_break;
                    current.paragraphs.push(p);
                    state = State::Idle;
                }
                State::InTable(stack) => {
                    if let Some(builder) = stack.last_mut() {
                        builder.set_pending_text_with_shapes(
                            text,
                            line_ps,
                            line_cs,
                            line_segs,
                        );
                    }
                }
                State::Idle => {
                    // Bare TEXT without a preceding PARAGRAPH marker
                    // — treat as a body paragraph so prose isn't
                    // dropped on the floor.
                    current
                        .paragraphs
                        .push(make_paragraph(0, text, line_ps, line_cs, line_segs));
                }
            }
            continue;
        }
        // FIGURE, CAPTION, EQUATION, BREAK… anything else falls
        // through silently for now.
    }

    flush_state(&mut state, &mut current);
    if !in_assets {
        // The assets-footer transition already pushed the body
        // section. A second push here would land an empty trailing
        // section.
        sections.push(current);
    }
    let _ = pending_asset; // drop in case ASSET[…] had no following DATA

    // Drop empty leading sections from the SECTION-flush boundary —
    // the loop pushes one Section per SECTION marker, so the first
    // entry is always the empty pre-section buffer.
    if sections.first().is_some_and(|s| s.paragraphs.is_empty())
        && sections.len() > 1
    {
        sections.remove(0);
    }
    if sections.is_empty() {
        sections.push(Section::default());
    }
    doc.sections = sections;

    // Reparse the verbatim-restored Contents/header.xml so doc_info
    // carries the full font/border/charShape/paraShape/style tables
    // that the surgical writer keys off. Without this the importer's
    // doc_info only holds the synthesised heading slots, and the
    // header rewriter trims every charPr/paraPr beyond IR.len() — a
    // 333KB header.xml ships at ~64KB on round-trip.
    if let Some(header_bytes) = doc.unknown_streams.get("Contents/header.xml") {
        if let Ok(hdr) = crate::hwpx::header_xml::parse_header_xml(header_bytes) {
            doc.doc_info.font_faces = hdr.font_faces;
            doc.doc_info.border_fills = hdr.border_fills;
            doc.doc_info.char_shapes = hdr.char_shapes;
            doc.doc_info.para_shapes = hdr.para_shapes;
            doc.doc_info.styles = hdr.styles;
        }
    }

    // Verify-gate: `SECTION_BYTES` freezes a section for byte-equal
    // replay, but the body records above it may have been edited after
    // export. Replaying the frozen bytes would silently discard those
    // edits. Compare the frozen XML's text against the typed
    // paragraphs' text (whitespace/object-marker insensitive); on
    // mismatch drop the verbatim cache so the writer rebuilds the
    // section from the edited paragraphs. HWP5 binary caches are left
    // alone — no cheap text diff exists for them.
    for section in &mut doc.sections {
        let Some(bytes) = section.stream_bytes.as_deref() else {
            continue;
        };
        if !crate::hwpx::writer::looks_like_xml(bytes) {
            continue;
        }
        let Ok(frozen) = crate::hwpx::section_xml::parse_section_xml(bytes) else {
            continue;
        };
        if comparable_text(&frozen.paragraphs) != comparable_text(&section.paragraphs) {
            section.stream_bytes = None;
            // Salvage the original page geometry before discarding
            // the frozen bytes — the rebuilt section must keep the
            // source margins / gutter / header-footer heights, or
            // the changed body width re-wraps every line. (Also
            // rescues md files exported before `sec_pr=` carry.)
            if section.sec_pr_xml.is_none() {
                section.sec_pr_xml = frozen.sec_pr_xml.clone();
            }
            // Table-level layout props (frame fill, inner padding,
            // cell gap) from md files exported before those attrs
            // were carried. Positional zip in document order — text
            // edits don't add or remove tables, so a count match
            // means the mapping is sound; on mismatch skip rather
            // than misattribute.
            let mut frozen_tables = Vec::new();
            collect_tables(&frozen.paragraphs, &mut frozen_tables);
            let mut typed_count = Vec::new();
            collect_tables(&section.paragraphs, &mut typed_count);
            if frozen_tables.len() == typed_count.len() {
                let mut idx = 0usize;
                transplant_table_props(&mut section.paragraphs, &frozen_tables, &mut idx);
            }
            // Re-insert the source's empty paragraphs (blank lines —
            // vertical rhythm) and copy per-paragraph header bits
            // (forced page breaks, style ids) the md didn't carry.
            // Rescues md files exported before those records existed;
            // a no-op for current exports, which carry them inline.
            align_paragraphs(&frozen.paragraphs, &mut section.paragraphs);
            // Line-layout cache goes with it. `vertpos` is cumulative
            // within its list (body flow / table cell), so any edit
            // that changes line count invalidates every following
            // paragraph's segments too — wipe the whole section and
            // let Hancom re-run layout (it does so for paragraphs
            // without a `<hp:linesegarray>`; the writer omits the
            // element when segments are empty).
            wipe_line_segments(&mut section.paragraphs);
        }
    }

    Ok(doc)
}

/// Pre-order (document order) walk collecting every table, nested
/// ones included, along with the paragraph that anchors it. Used by
/// the verify-gate to pair frozen tables with their typed
/// counterparts positionally.
fn collect_tables<'a>(
    paragraphs: &'a [Paragraph],
    out: &mut Vec<(&'a Paragraph, &'a TableControl)>,
) {
    for p in paragraphs {
        for c in &p.controls {
            if let ControlKind::Table(t) = &c.kind {
                out.push((p, t));
                for cell in &t.cells {
                    collect_tables(&cell.paragraphs, out);
                }
            }
        }
    }
}

/// Same pre-order walk on the mutable side, copying table-level
/// layout props from the positionally-matching frozen table wherever
/// the typed value is still the "not carried" default, and the
/// anchor paragraph's shape ids onto the typed wrapper — an inline
/// (treatAsChar) table's spacing comes from its anchor's paraShape,
/// so losing it to slot 0 changes the page's total height. Explicit
/// md attrs win — only defaults are filled in.
fn transplant_table_props(
    paragraphs: &mut [Paragraph],
    frozen: &[(&Paragraph, &TableControl)],
    idx: &mut usize,
) {
    for p in paragraphs {
        let mut frozen_anchor: Option<&Paragraph> = None;
        for c in &mut p.controls {
            if let ControlKind::Table(t) = &mut c.kind {
                if let Some((owner, src)) = frozen.get(*idx) {
                    *idx += 1;
                    frozen_anchor = Some(owner);
                    if t.border_fill_id == 0 {
                        t.border_fill_id = src.border_fill_id;
                    }
                    if t.padding == [0; 4] {
                        t.padding = src.padding;
                    }
                    if t.cell_spacing == 0 {
                        t.cell_spacing = src.cell_spacing;
                    }
                }
                for cell in &mut t.cells {
                    transplant_table_props(&mut cell.paragraphs, frozen, idx);
                }
            }
        }
        if let Some(owner) = frozen_anchor {
            // Shape bits only — page breaks are transplanted by the
            // text-anchored alignment, which self-corrects around
            // edits; the positional table zip can slip by one in
            // deeply nested regions and would misplace a break.
            copy_shape_bits(owner, p);
        }
    }
}

/// Blank paragraph: no controls and no visible text. These are the
/// blank lines that give a document its vertical rhythm; md exports
/// prior to 2026-07-02 dropped them entirely.
fn is_blank(p: &Paragraph) -> bool {
    p.controls.is_empty()
        && p
            .text
            .chars()
            .all(|c| c.is_whitespace() || c == '\u{FFFC}')
}

fn strip_visible(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{FFFC}')
        .collect()
}

/// Do a frozen and a typed paragraph carry the same visible text?
/// Old exports flattened `<hp:lineBreak/>` paragraphs to their first
/// line — accept that prefix as equal so those still anchor.
fn same_text(f: &Paragraph, t: &Paragraph) -> bool {
    let fs = strip_visible(&f.text);
    let ts = strip_visible(&t.text);
    if fs == ts {
        return true;
    }
    !ts.is_empty()
        && f.text.contains('\n')
        && strip_visible(f.text.split('\n').next().unwrap_or("")) == ts
}

/// Anchored alignment of frozen (source) paragraphs against typed
/// (md-imported) paragraphs. Pairs on matching visible text and
/// resyncs across local insertions/deletions with a bounded
/// lookahead, so an AI edit desyncs at most its own neighbourhood —
/// a purely positional walk used to smear page breaks and blank
/// lines onto the wrong paragraphs for the entire rest of the
/// document. Re-inserts blank paragraphs the md didn't carry and
/// copies per-paragraph header bits onto each pair; recurses into
/// paired tables cell-by-cell.
fn align_paragraphs(frozen: &[Paragraph], typed: &mut Vec<Paragraph>) {
    const WINDOW: usize = 5;
    let mut i = 0usize;
    let mut j = 0usize;
    while i < frozen.len() {
        let f = &frozen[i];
        if is_blank(f) {
            if typed.get(j).is_some_and(is_blank) {
                copy_header_bits(f, &mut typed[j]);
            } else {
                let mut clone = f.clone();
                // Layout cache is stale by definition here (the
                // section was edited); the gate wipes the rest.
                clone.line_segments.clear();
                typed.insert(j, clone);
            }
            i += 1;
            j += 1;
            continue;
        }
        if j >= typed.len() {
            break;
        }
        if same_text(f, &typed[j]) {
            pair_paragraphs(f, &mut typed[j]);
            i += 1;
            j += 1;
            continue;
        }
        // Mismatch — try to resync within a small window before
        // declaring this an in-place edit.
        let t_skip = (1..=WINDOW).find(|k| {
            typed
                .get(j + k)
                .is_some_and(|t| !is_blank(t) && same_text(f, t))
        });
        let f_skip = (1..=WINDOW).find(|k| {
            frozen
                .get(i + k)
                .is_some_and(|fk| !is_blank(fk) && same_text(fk, &typed[j]))
        });
        match (t_skip, f_skip) {
            // Typed inserted paragraphs (AI additions) — keep them,
            // don't scatter frozen blanks into the inserted run.
            (Some(tk), None) => j += tk,
            // Frozen paragraphs deleted/replaced by the edit.
            (None, Some(fk)) => i += fk,
            (Some(tk), Some(fk)) => {
                if tk <= fk {
                    j += tk;
                } else {
                    i += fk;
                }
            }
            // In-place edit — still the same paragraph slot.
            (None, None) => {
                pair_paragraphs(f, &mut typed[j]);
                i += 1;
                j += 1;
            }
        }
    }
}

/// Copy everything the md didn't carry from a frozen paragraph onto
/// its aligned typed counterpart.
fn pair_paragraphs(f: &Paragraph, t: &mut Paragraph) {
    copy_header_bits(f, t);
    // Restore text lost after `<hp:lineBreak/>` in older exports:
    // their md carried only the first line, so the tail never
    // reached the editing AI. Restore only when the typed text is
    // exactly the frozen text's first line (whitespace-insensitive)
    // — an edited prefix fails the match and the edit wins.
    if f.text.contains('\n') {
        let first_line = f.text.split('\n').next().unwrap_or("");
        if strip_visible(&t.text) == strip_visible(first_line) {
            t.text = f.text.clone();
            t.char_shape_runs = f.char_shape_runs.clone();
        }
    }
    align_tables(f, t);
}

fn copy_header_bits(f: &Paragraph, t: &mut Paragraph) {
    if f.header.page_break_before {
        t.header.page_break_before = true;
    }
    copy_shape_bits(f, t);
}

fn copy_shape_bits(f: &Paragraph, t: &mut Paragraph) {
    if t.header.style_id == 0 {
        t.header.style_id = f.header.style_id;
    }
    // Slot 0 on the typed side means "not carried" (older md, or a
    // wrapper the importer synthesised) — restore the source shape.
    if t.header.para_shape_id == 0 && f.header.para_shape_id != 0 {
        t.header.para_shape_id = f.header.para_shape_id;
    }
    if t.char_shape_runs.is_empty() && !f.char_shape_runs.is_empty() {
        t.char_shape_runs = f.char_shape_runs.clone();
    }
}

/// Pair the tables of two aligned paragraphs in order and align each
/// pair's cells by (row, col).
fn align_tables(f: &Paragraph, t: &mut Paragraph) {
    let f_tables: Vec<&TableControl> = f
        .controls
        .iter()
        .filter_map(|c| match &c.kind {
            ControlKind::Table(ft) => Some(ft),
            _ => None,
        })
        .collect();
    let mut ti = 0usize;
    for c in &mut t.controls {
        if let ControlKind::Table(tt) = &mut c.kind {
            if let Some(ft) = f_tables.get(ti) {
                ti += 1;
                for cell in &mut tt.cells {
                    if let Some(fc) = ft
                        .cells
                        .iter()
                        .find(|c2| c2.row == cell.row && c2.col == cell.col)
                    {
                        align_paragraphs(&fc.paragraphs, &mut cell.paragraphs);
                        cell.para_count = cell.paragraphs.len() as i32;
                    }
                }
            }
        }
    }
}

/// Recursively clear captured line segments (tables included) so the
/// writer omits `<hp:linesegarray>` and the viewer re-lays-out.
fn wipe_line_segments(paragraphs: &mut [Paragraph]) {
    for p in paragraphs {
        p.line_segments.clear();
        for control in &mut p.controls {
            if let ControlKind::Table(table) = &mut control.kind {
                for cell in &mut table.cells {
                    wipe_line_segments(&mut cell.paragraphs);
                }
            }
        }
    }
}

/// Concatenated paragraph text (tables included, depth-first) with
/// whitespace and U+FFFC object markers stripped — the equality key
/// the verify-gate uses to decide whether the body records still
/// match a frozen `SECTION_BYTES` cache. Whitespace-only edits are
/// deliberately invisible to this key: false negatives there are
/// harmless next to losing byte-equal replay on every roundtrip.
fn comparable_text(paragraphs: &[Paragraph]) -> String {
    fn walk(paragraphs: &[Paragraph], out: &mut String) {
        for p in paragraphs {
            out.extend(
                p.text
                    .chars()
                    .filter(|c| !c.is_whitespace() && *c != '\u{FFFC}'),
            );
            for control in &p.controls {
                if let ControlKind::Table(table) = &control.kind {
                    for cell in &table.cells {
                        walk(&cell.paragraphs, out);
                    }
                }
            }
        }
    }
    let mut out = String::new();
    walk(paragraphs, &mut out);
    out
}

enum State {
    Idle,
    /// `explicit_level` is `Some(N)` when the PARAGRAPH record
    /// carried `level=N`. The TEXT branch falls back to a `# `-
    /// prefix scan when this is `None`. `para_shape_id` /
    /// `char_shape_id` are the slot ids the PARAGRAPH record
    /// carries — applied to the resulting `Paragraph::header` /
    /// `char_shape_runs` so HWPX `paraPrIDRef` / `charPrIDRef`
    /// route to the matching shape entries on round-trip.
    ExpectingParagraphText {
        explicit_level: Option<u8>,
        para_shape_id: u32,
        char_shape_id: u32,
        /// Line layout from the PARAGRAPH record's `lineseg=` attr,
        /// applied to the paragraph so multi-line bodies keep their
        /// per-line geometry instead of collapsing to a single seed.
        line_segments: Vec<LineSegment>,
        /// `page_break=1` — start this paragraph on a new page.
        page_break: bool,
    },
    /// Builder stack: `last()` is the innermost table. A `TABLE[`
    /// record while already in a table pushes a nested builder; its
    /// `END TABLE` pops it and attaches the finished table to the
    /// parent's current cell. A flat single-builder state here used
    /// to *finish* the outer table on nested `TABLE[` — flattening
    /// every nested form table into top-level siblings and dropping
    /// the outer table's remaining cells.
    InTable(Vec<LlmTableBuilder>),
}

/// Held between an `ASSET[…]` line and its following `DATA: …`
/// line so the data URI decode happens with the asset's id /
/// bin_id metadata in hand.
struct PendingAsset {
    /// Reserved for future use — `BinaryEntry.id` is currently the
    /// only field needed downstream, but keeping `bin_id` here means
    /// any later cross-validation (PictureControl.bin_id matching
    /// ASSET.bin_id) doesn't need a re-parse.
    #[allow(dead_code)]
    bin_id: Option<u16>,
    source_id: String,
}

#[derive(Default)]
struct LlmTableBuilder {
    rows: u16,
    cols: u16,
    cells: Vec<TableCell>,
    pending: Option<PendingCell>,
    /// Table-level `borderFillIDRef` from the `TABLE[border_fill=N]`
    /// attribute. 0 = no table-level border (cells provide their own).
    border_fill_id: u16,
    /// Inner cell padding `[left, right, top, bottom]` from
    /// `in_margin=l:r:t:b`; zeros mean "not carried" (writer falls
    /// back to the Hancom default).
    padding: [i16; 4],
    /// Cell gap from `cell_spacing=N`.
    cell_spacing: i16,
    /// Anchor-paragraph properties consumed from the PARAGRAPH record
    /// immediately preceding this table's `TABLE[` record:
    /// `(para_shape, char_shape, page_break)`. Applied to the wrapper
    /// paragraph at END TABLE — the anchor's paraShape decides the
    /// spacing around an inline table.
    anchor: Option<(u32, u32, bool)>,
}

struct PendingCell {
    row: u16,
    col: u16,
    row_span: u16,
    col_span: u16,
    /// `border_fill` attribute from the CELL record. Falls back to 1
    /// (skeleton's plain SOLID 0.12mm) when the attr is missing —
    /// keeps GFM-imported tables visible while letting LLM-mode
    /// round-trip preserve the source's per-cell border style.
    border_fill_id: u16,
    /// Real cell geometry (HWPUNIT) from the CELL record. `0` when the
    /// attr is absent (older exports / GFM) → `apply_defaults` then
    /// distributes the page width evenly as before.
    width_hwpu: u32,
    height_hwpu: u32,
    text_width_hwpu: u32,
}

impl LlmTableBuilder {
    fn set_pending_text_with_shapes(
        &mut self,
        text: String,
        ps: u32,
        cs: u32,
        segs: Vec<LineSegment>,
    ) {
        if let Some(p) = self.pending.take() {
            self.push_cell(p, text, ps, cs, segs);
        } else if let Some(cell) = self.cells.last_mut() {
            // Second and later TEXT records of the same CELL block —
            // each is its own paragraph. These used to be silently
            // dropped, which deleted every multi-paragraph cell's
            // body past the first line.
            let mut para = Paragraph::default();
            para.text = text;
            para.line_segments = segs;
            para.header.para_shape_id = ps as u16;
            if cs != 0 {
                para.char_shape_runs.push(CharShapeRun {
                    start: 0,
                    char_shape_id: cs,
                });
            }
            cell.paragraphs.push(para);
            cell.para_count = cell.paragraphs.len() as i32;
        }
    }
    /// Append a pre-built paragraph (nested table / figure wrapper)
    /// to the current cell, materialising a pending CELL record if
    /// its first content is structural rather than text.
    fn attach_paragraph(&mut self, para: Paragraph) {
        if let Some(p) = self.pending.take() {
            self.push_cell_with_paragraph(p, para);
        } else if let Some(cell) = self.cells.last_mut() {
            cell.paragraphs.push(para);
            cell.para_count = cell.paragraphs.len() as i32;
        }
    }
    fn push_cell_with_paragraph(&mut self, p: PendingCell, para: Paragraph) {
        self.cells.push(TableCell {
            row: p.row,
            col: p.col,
            row_span: p.row_span,
            col_span: p.col_span,
            para_count: 1,
            border_fill_id: p.border_fill_id,
            width_hwpu: p.width_hwpu,
            height_hwpu: p.height_hwpu,
            text_width_hwpu: p.text_width_hwpu,
            paragraphs: vec![para],
            ..TableCell::default()
        });
        self.rows = self.rows.max(p.row + p.row_span);
        self.cols = self.cols.max(p.col + p.col_span);
    }
    fn flush_pending(&mut self) {
        if let Some(p) = self.pending.take() {
            self.push_cell(p, String::new(), 0, 0, Vec::new());
        }
    }
    fn push_cell(
        &mut self,
        p: PendingCell,
        text: String,
        ps: u32,
        cs: u32,
        segs: Vec<LineSegment>,
    ) {
        let mut para = Paragraph::default();
        para.text = text;
        para.line_segments = segs;
        para.header.para_shape_id = ps as u16;
        if cs != 0 {
            para.char_shape_runs.push(CharShapeRun {
                start: 0,
                char_shape_id: cs,
            });
        }
        self.cells.push(TableCell {
            row: p.row,
            col: p.col,
            row_span: p.row_span,
            col_span: p.col_span,
            para_count: 1,
            border_fill_id: p.border_fill_id,
            width_hwpu: p.width_hwpu,
            height_hwpu: p.height_hwpu,
            text_width_hwpu: p.text_width_hwpu,
            paragraphs: vec![para],
            ..TableCell::default()
        });
        self.rows = self.rows.max(p.row + p.row_span);
        self.cols = self.cols.max(p.col + p.col_span);
    }
    fn finish(mut self) -> TableControl {
        self.flush_pending();
        // Distribute the default A4 text-region width evenly across
        // columns so HWPX viewers don't collapse 0-width cells.
        // Heights default to a single text-row.
        super::cell_sizes::apply_defaults(&mut self.cells, self.cols);
        let row_cell_counts = vec![self.cols; self.rows as usize];
        TableControl {
            rows: self.rows,
            cols: self.cols,
            row_cell_counts,
            cells: self.cells,
            border_fill_id: self.border_fill_id,
            padding: self.padding,
            cell_spacing: self.cell_spacing,
            ..TableControl::default()
        }
    }
}

fn flush_state(state: &mut State, section: &mut Section) {
    match std::mem::replace(state, State::Idle) {
        State::Idle => {}
        State::ExpectingParagraphText {
            explicit_level,
            para_shape_id,
            char_shape_id,
            line_segments,
            page_break,
        } => {
            // PARAGRAPH record with no TEXT line — a deliberately
            // empty paragraph. Each is a real blank line in the
            // source document; materialise it so vertical spacing
            // (and where page breaks fall) survives the round-trip.
            let level = explicit_level.unwrap_or(0);
            let mut p = make_paragraph(
                level,
                String::new(),
                para_shape_id,
                char_shape_id,
                line_segments,
            );
            p.header.page_break_before = page_break;
            section.paragraphs.push(p);
        }
        State::InTable(mut stack) => {
            // Mid-table flush at section boundary — collapse the
            // builder stack innermost-first so nested cells aren't
            // lost, then land the outermost table in the section.
            while let Some(builder) = stack.pop() {
                let anchor = builder.anchor;
                let table = builder.finish();
                let wrapper = table_wrapper(table, anchor);
                if let Some(parent) = stack.last_mut() {
                    parent.attach_paragraph(wrapper);
                } else {
                    section.paragraphs.push(wrapper);
                }
            }
        }
    }
}

/// Build the anchor paragraph a finished table lives in, applying
/// any shape ids consumed from the PARAGRAPH record that preceded
/// the table.
fn table_wrapper(table: TableControl, anchor: Option<(u32, u32, bool)>) -> Paragraph {
    let mut wrapper = Paragraph::default();
    wrapper.text = "\u{FFFC}".into();
    wrapper.controls.push(Control {
        kind: ControlKind::Table(table),
        caption_text: None,
    });
    if let Some((ps, cs, pb)) = anchor {
        wrapper.header.para_shape_id = ps as u16;
        wrapper.header.page_break_before = pb;
        if cs != 0 {
            wrapper.char_shape_runs.push(CharShapeRun {
                start: 0,
                char_shape_id: cs,
            });
        }
    }
    wrapper
}

/// Pull the comma-separated `key=value` attribute set out of a
/// `KIND[…]` record line. Tolerates whitespace and stops scanning at
/// the first `]`. Values can't contain `,` or `]` (matches the
/// exporter's emit guarantees).
fn parse_attrs(line: &str) -> AttrMap {
    let mut map = HashMap::new();
    let Some(open) = line.find('[') else {
        return AttrMap(map);
    };
    let rest = &line[open + 1..];
    let body = rest.split(']').next().unwrap_or("");
    for piece in body.split(',') {
        let piece = piece.trim();
        if let Some((k, v)) = piece.split_once('=') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    AttrMap(map)
}

struct AttrMap(HashMap<String, String>);

impl AttrMap {
    fn get_int(&self, key: &str) -> Option<i64> {
        self.0.get(key).and_then(|v| v.parse::<i64>().ok())
    }
}

/// Build a body paragraph (`level=0`) or a heading paragraph
/// (`level=1..=6`). Heading paragraphs reference the matching
/// `style_id` / `para_shape_id` / `char_shape_id` so HWPX viewers
/// render with the heading style/font from the synthesised
/// DocInfo table.
fn make_paragraph(
    level: u8,
    text: String,
    para_shape: u32,
    char_shape: u32,
    line_segments: Vec<LineSegment>,
) -> Paragraph {
    let mut p = Paragraph::default();
    p.text = text;
    p.line_segments = line_segments;
    // PARAGRAPH record's `para_shape` overrides the heading-level
    // fallback when present. Heading paragraphs from `# `-prefix MD
    // (no doc_info) keep using `level` as the slot id since
    // `style_synth` synthesises one paraShape per heading level.
    let para_shape_id = if para_shape != 0 {
        para_shape as u16
    } else {
        level as u16
    };
    p.header = ParagraphHeader {
        style_id: level,
        para_shape_id,
        ..ParagraphHeader::default()
    };
    let char_shape_id = if char_shape != 0 {
        char_shape
    } else if level > 0 {
        level as u32
    } else {
        0
    };
    if char_shape_id != 0 || level > 0 {
        p.char_shape_runs.push(CharShapeRun {
            start: 0,
            char_shape_id,
        });
    }
    p
}

/// Decide the heading level for a TEXT line. Explicit `level=N`
/// from the PARAGRAPH record wins; otherwise check for a Markdown
/// `# ` … `###### ` prefix on the text and strip it. Returns the
/// `(level, body)` pair after any prefix removal.
fn resolve_heading(explicit: Option<u8>, text: &str) -> (u8, String) {
    if let Some(level) = explicit {
        if level > 0 {
            return (level, text.to_string());
        }
    }
    if let Some((level, rest)) = strip_atx_heading_prefix(text) {
        return (level, rest.to_string());
    }
    (explicit.unwrap_or(0), text.to_string())
}

/// `"## Foo"` → `Some((2, "Foo"))`. Returns `None` when the line
/// isn't an ATX heading.
fn strip_atx_heading_prefix(text: &str) -> Option<(u8, &str)> {
    let mut hash_count = 0u8;
    for c in text.chars() {
        if c == '#' {
            hash_count += 1;
            if hash_count > 6 {
                return None;
            }
        } else {
            break;
        }
    }
    if hash_count == 0 {
        return None;
    }
    let after_hashes = &text[hash_count as usize..];
    let body = after_hashes.strip_prefix(' ')?;
    Some((hash_count, body))
}

/// Strip `TEXT: ` or `TEXT[…]: ` prefix; returns `None` for non-TEXT
/// lines.
fn strip_text_prefix(line: &str) -> Option<&str> {
    if let Some(rest) = line.strip_prefix("TEXT: ") {
        return Some(rest);
    }
    // Empty-body records: the caller trims trailing whitespace off
    // the raw line, so `TEXT: ` / `TEXT[…]: ` arrive as `TEXT:` /
    // `TEXT[…]:`. They mark deliberately empty paragraphs (blank
    // lines that give tall form cells their height) — return the
    // empty string rather than rejecting the record.
    if line == "TEXT:" {
        return Some("");
    }
    let rest = line.strip_prefix("TEXT[")?;
    if let Some(close) = rest.find("]: ") {
        return Some(&rest[close + 3..]);
    }
    if rest.ends_with("]:") {
        return Some("");
    }
    None
}

/// Inverse of `export::markdown_llm::escape_text` — restore `\n`
/// (Shift+Enter line breaks, `<hp:lineBreak/>`), `\t`, and literal
/// backslashes in TEXT record bodies.
fn unescape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Parse a `TEXT[par-PATH,para_shape=N,char_shape=N,lineseg=…]: text`
/// line into `(text, para_shape, char_shape, line_segments)`.
/// `para_shape` / `char_shape` are `0` and `line_segments` empty when
/// the attrs are missing (older exports). Returns `None` for non-TEXT
/// lines.
fn parse_text_line(line: &str) -> Option<(&str, u32, u32, Vec<LineSegment>)> {
    if let Some(rest) = line.strip_prefix("TEXT: ") {
        return Some((rest, 0, 0, Vec::new()));
    }
    if line == "TEXT:" {
        return Some(("", 0, 0, Vec::new()));
    }
    let rest = line.strip_prefix("TEXT[")?;
    // Empty-body form: the import loop trims each raw line, so an
    // empty cell paragraph's `TEXT[…]: ` arrives as `TEXT[…]:`.
    let (attrs_text, body) = match rest.find("]: ") {
        Some(close) => (&rest[..close], &rest[close + 3..]),
        None if rest.ends_with("]:") => (&rest[..rest.len() - 2], ""),
        None => return None,
    };
    let mut ps = 0u32;
    let mut cs = 0u32;
    let mut segs = Vec::new();
    for kv in attrs_text.split(',') {
        let kv = kv.trim();
        if let Some(v) = kv.strip_prefix("para_shape=") {
            ps = v.parse().unwrap_or(0);
        } else if let Some(v) = kv.strip_prefix("char_shape=") {
            cs = v.parse().unwrap_or(0);
        } else if let Some(v) = kv.strip_prefix("lineseg=") {
            segs = crate::lineseg_codec::decode(v);
        }
    }
    Some((body, ps, cs, segs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_llm_format_by_leading_sigil() {
        assert!(looks_like_llm_format("SECTION[id=sec-0]\n"));
        assert!(looks_like_llm_format("\n\nSECTION[id=sec-0]\n"));
        assert!(!looks_like_llm_format("# Heading\n"));
        assert!(!looks_like_llm_format("Hello body."));
        assert!(!looks_like_llm_format(""));
    }

    #[test]
    fn empty_section_yields_one_empty_section() {
        let doc = from_llm_markdown("SECTION[id=sec-0]\n").expect("parse");
        assert_eq!(doc.sections.len(), 1);
        assert!(doc.sections[0].paragraphs.is_empty());
    }

    #[test]
    fn paragraph_text_collected() {
        let src = "SECTION[id=sec-0]\n\nPARAGRAPH[id=par-s0-p0]\nTEXT: 안녕\n";
        let doc = from_llm_markdown(src).expect("parse");
        assert_eq!(doc.sections[0].paragraphs.len(), 1);
        assert_eq!(doc.sections[0].paragraphs[0].text, "안녕");
    }

    #[test]
    fn simple_2x2_table_lands_with_correct_dims() {
        let src = "\
SECTION[id=sec-0]

TABLE[id=t,rows=2,cols=2]
CELL[id=c00,row=0,col=0,rowspan=1,colspan=1]
TEXT[c00-p0]: A
CELL[id=c01,row=0,col=1,rowspan=1,colspan=1]
TEXT[c01-p0]: B
CELL[id=c10,row=1,col=0,rowspan=1,colspan=1]
TEXT[c10-p0]: 1
CELL[id=c11,row=1,col=1,rowspan=1,colspan=1]
TEXT[c11-p0]: 2
END TABLE[t]
";
        let doc = from_llm_markdown(src).expect("parse");
        let table = match &doc.sections[0].paragraphs[0].controls[0].kind {
            ControlKind::Table(t) => t,
            _ => panic!("expected table"),
        };
        assert_eq!(table.rows, 2);
        assert_eq!(table.cols, 2);
        assert_eq!(table.cells.len(), 4);
    }

    #[test]
    fn cell_rowspan_and_colspan_extend_table_dims() {
        let src = "\
SECTION[id=sec-0]

TABLE[id=t,rows=2,cols=3]
CELL[id=c,row=0,col=0,rowspan=2,colspan=3]
TEXT[c-p0]: merged
END TABLE[t]
";
        let doc = from_llm_markdown(src).expect("parse");
        let table = match &doc.sections[0].paragraphs[0].controls[0].kind {
            ControlKind::Table(t) => t,
            _ => panic!(),
        };
        assert_eq!(table.rows, 2);
        assert_eq!(table.cols, 3);
        assert_eq!(table.cells[0].row_span, 2);
        assert_eq!(table.cells[0].col_span, 3);
        assert_eq!(table.cells[0].paragraphs[0].text, "merged");
    }

    #[test]
    fn paragraph_then_table_then_paragraph_preserves_order() {
        let src = "\
SECTION[id=sec-0]

PARAGRAPH[id=p0]
TEXT: 첫 단락

TABLE[id=t,rows=1,cols=1]
CELL[id=c,row=0,col=0,rowspan=1,colspan=1]
TEXT[c-p0]: 셀
END TABLE[t]

PARAGRAPH[id=p1]
TEXT: 마지막 단락
";
        let doc = from_llm_markdown(src).expect("parse");
        let ps = &doc.sections[0].paragraphs;
        assert_eq!(ps.len(), 3);
        assert_eq!(ps[0].text, "첫 단락");
        assert_eq!(ps[1].text, "\u{FFFC}");
        assert!(matches!(ps[1].controls[0].kind, ControlKind::Table(_)));
        assert_eq!(ps[2].text, "마지막 단락");
    }

    #[test]
    fn parse_attrs_handles_whitespace_and_extra_keys() {
        let attrs = parse_attrs("CELL[ row = 3, col=4, rowspan = 2, colspan=5, role=header ]");
        assert_eq!(attrs.get_int("row"), Some(3));
        assert_eq!(attrs.get_int("col"), Some(4));
        assert_eq!(attrs.get_int("rowspan"), Some(2));
        assert_eq!(attrs.get_int("colspan"), Some(5));
        assert_eq!(attrs.0.get("role").map(String::as_str), Some("header"));
    }

    #[test]
    fn strip_text_prefix_handles_both_forms() {
        assert_eq!(strip_text_prefix("TEXT: hello"), Some("hello"));
        assert_eq!(strip_text_prefix("TEXT[some-id]: hello"), Some("hello"));
        assert_eq!(strip_text_prefix("not text"), None);
    }
}
