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
            };
            continue;
        }

        if trimmed.starts_with("TABLE[") {
            flush_state(&mut state, &mut current);
            let attrs = parse_attrs(trimmed);
            let mut builder = LlmTableBuilder::default();
            builder.border_fill_id = attrs.get_int("border_fill").unwrap_or(0).max(0) as u16;
            state = State::InTable(builder);
            continue;
        }

        if trimmed.starts_with("END TABLE") {
            if let State::InTable(builder) = std::mem::replace(&mut state, State::Idle) {
                let table = builder.finish();
                let mut wrapper = Paragraph::default();
                wrapper.text = "\u{FFFC}".into();
                wrapper.controls.push(Control {
                    kind: ControlKind::Table(table),
                    caption_text: None,
                });
                current.paragraphs.push(wrapper);
            }
            continue;
        }

        if trimmed.starts_with("FIGURE[") {
            flush_state(&mut state, &mut current);
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
            current.paragraphs.push(wrapper);
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
            if let State::InTable(builder) = &mut state {
                builder.flush_pending();
                builder.pending = Some(PendingCell {
                    row, col, row_span, col_span, border_fill_id,
                    width_hwpu, height_hwpu, text_width_hwpu,
                });
            }
            continue;
        }

        if let Some((text, line_ps, line_cs, line_segs)) = parse_text_line(trimmed) {
            match &mut state {
                State::ExpectingParagraphText {
                    explicit_level,
                    para_shape_id,
                    char_shape_id,
                    line_segments,
                } => {
                    let (level, body) = resolve_heading(*explicit_level, text);
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
                    current.paragraphs.push(make_paragraph(level, body, ps, cs, segs));
                    state = State::Idle;
                }
                State::InTable(builder) => {
                    builder.set_pending_text_with_shapes(
                        text.to_string(),
                        line_ps,
                        line_cs,
                        line_segs,
                    );
                }
                State::Idle => {
                    // Bare TEXT without a preceding PARAGRAPH marker
                    // — treat as a body paragraph so prose isn't
                    // dropped on the floor.
                    current
                        .paragraphs
                        .push(make_paragraph(0, text.to_string(), line_ps, line_cs, line_segs));
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
    },
    InTable(LlmTableBuilder),
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
        }
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
            ..TableControl::default()
        }
    }
}

fn flush_state(state: &mut State, section: &mut Section) {
    match std::mem::replace(state, State::Idle) {
        State::Idle | State::ExpectingParagraphText { .. } => {}
        State::InTable(builder) => {
            // Mid-table flush at section boundary — emit what we
            // have so cells aren't lost.
            let table = builder.finish();
            let mut wrapper = Paragraph::default();
            wrapper.text = "\u{FFFC}".into();
            wrapper.controls.push(Control {
                kind: ControlKind::Table(table),
                caption_text: None,
            });
            section.paragraphs.push(wrapper);
        }
    }
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
    let rest = line.strip_prefix("TEXT[")?;
    let close = rest.find("]: ")?;
    Some(&rest[close + 3..])
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
    let rest = line.strip_prefix("TEXT[")?;
    let close = rest.find("]: ")?;
    let attrs_text = &rest[..close];
    let body = &rest[close + 3..];
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
