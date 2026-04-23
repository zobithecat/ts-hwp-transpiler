//! `/BodyText/Section{N}` — compressed record tree (same TLV + DEFLATE
//! framing as `/DocInfo`, but with paragraph-scoped tags).
//!
//! Records carry a `level` field indicating nesting depth. Section-level
//! records (page/footnote/border defs) live at level 0; a top-level
//! paragraph's PARA_HEADER is at level 0 with its children at level 1;
//! inside a table, a cell LIST_HEADER is at the control's level + 1, and
//! the cell's paragraphs are one level deeper still.
//!
//! We parse with recursive descent on `level` — `parse_paragraph` consumes
//! every record at its child level, and when it encounters a CTRL_HEADER
//! "tbl " it dives one level deeper to read the TABLE + LIST_HEADER stream
//! before returning.

use std::iter::Peekable;
use std::vec::IntoIter;

use hwp_transpiler_core::ir::{
    ControlKind, IrError, Paragraph, PictureControl, Record, Section, TableControl,
};

use super::{
    ctrl_header, doc_info, gso_picture, list_header, paragraph_char_shape, paragraph_header,
    paragraph_line_seg, paragraph_text, record, table,
};

pub const STREAM_PREFIX: &str = "/BodyText/Section";

pub mod tag {
    pub const PARA_HEADER: u16 = 0x0042;
    pub const PARA_TEXT: u16 = 0x0043;
    pub const PARA_CHAR_SHAPE: u16 = 0x0044;
    pub const PARA_LINE_SEG: u16 = 0x0045;
    pub const PARA_RANGE_TAG: u16 = 0x0046;
    pub const CTRL_HEADER: u16 = 0x0047;
    pub const LIST_HEADER: u16 = 0x0048;
    pub const PAGE_DEF: u16 = 0x0049;
    pub const FOOTNOTE_SHAPE: u16 = 0x004A;
    pub const PAGE_BORDER_FILL: u16 = 0x004B;
    pub const SHAPE_COMPONENT: u16 = 0x004C;
    pub const TABLE: u16 = 0x004D;
    pub const SHAPE_COMPONENT_PICTURE: u16 = 0x0055;
}

type RecIter = Peekable<IntoIter<Record>>;

pub fn parse_section(stream_bytes: &[u8], compressed: bool) -> Result<Section, IrError> {
    let decompressed = if compressed {
        doc_info::decompress(stream_bytes)?
    } else {
        stream_bytes.to_vec()
    };
    let records = record::parse_all(&decompressed)?;
    let mut iter = records.into_iter().peekable();

    let mut section = Section {
        stream_bytes: Some(stream_bytes.to_vec()),
        ..Section::default()
    };

    // Records before the first top-level PARA_HEADER are section-scoped.
    while let Some(r) = iter.peek() {
        if r.tag == tag::PARA_HEADER && r.level == 0 {
            break;
        }
        section.raw_records.push(iter.next().unwrap());
    }

    while iter.peek().is_some() {
        section.paragraphs.push(parse_paragraph(&mut iter, 0)?);
    }

    Ok(section)
}

/// Consume one paragraph from `iter` whose PARA_HEADER is at `level`.
fn parse_paragraph(iter: &mut RecIter, level: u16) -> Result<Paragraph, IrError> {
    let header_rec = iter
        .next()
        .ok_or_else(|| IrError::Invalid("expected PARA_HEADER, got EOS".into()))?;
    debug_assert_eq!(header_rec.tag, tag::PARA_HEADER);
    debug_assert_eq!(header_rec.level, level);

    let header = paragraph_header::parse(&header_rec)?;
    let mut para = Paragraph {
        header,
        ..Paragraph::default()
    };
    para.raw_records.push(header_rec);

    let child_lvl = level + 1;
    let mut pending_table: Option<usize> = None;
    let mut pending_picture: Option<PendingPicture> = None;

    while let Some(peek) = iter.peek() {
        // Scope exit: another top-level PARA_HEADER or a record shallower
        // than our children.
        if peek.tag == tag::PARA_HEADER && peek.level <= level {
            break;
        }
        if peek.level < child_lvl {
            break;
        }

        let rec = iter.next().unwrap();

        if rec.level == child_lvl {
            match rec.tag {
                tag::PARA_TEXT => {
                    para.text = paragraph_text::extract_text(&rec)?;
                }
                tag::PARA_CHAR_SHAPE => {
                    para.char_shape_runs = paragraph_char_shape::parse(&rec)?;
                }
                tag::PARA_LINE_SEG => {
                    para.line_segments = paragraph_line_seg::parse(&rec)?;
                }
                tag::CTRL_HEADER => {
                    let ctrl = ctrl_header::parse(&rec)?;
                    let code = match &ctrl.kind {
                        ControlKind::Unknown { code, .. } => Some(*code),
                        _ => None,
                    };
                    let display = code.as_ref().map(|c| ctrl_header::display_code(c));
                    let is_table = display.as_deref() == Some("tbl ");
                    let is_gso = display.as_deref() == Some("gso ");

                    para.controls.push(ctrl);
                    let ctrl_idx = para.controls.len() - 1;

                    pending_table = is_table.then_some(ctrl_idx);
                    pending_picture = if is_gso {
                        // CTRL_HEADER body holds the gso width/height; the
                        // shape kind is decided later by SHAPE_COMPONENT's
                        // gsoId.
                        let body = match &para.controls[ctrl_idx].kind {
                            ControlKind::Unknown { data, .. } => data.as_slice(),
                            _ => &[],
                        };
                        let (width_hwpu, height_hwpu) =
                            gso_picture::parse_gso_size(body).unwrap_or((0, 0));
                        Some(PendingPicture {
                            ctrl_idx,
                            width_hwpu,
                            height_hwpu,
                            is_pic_confirmed: false,
                            in_caption: false,
                            caption_parts: Vec::new(),
                        })
                    } else {
                        None
                    };
                }
                _ => {}
            }
        } else if rec.level == child_lvl + 1 && rec.tag == tag::TABLE {
            if let Some(idx) = pending_table.take() {
                let mut tc = table::parse(&rec)?;
                para.raw_records.push(rec);
                parse_table_cells(iter, &mut tc, child_lvl + 1)?;
                para.controls[idx].kind = ControlKind::Table(tc);
                continue;
            }
        } else if rec.level == child_lvl + 1 && rec.tag == tag::LIST_HEADER {
            // Optional caption container inside a gso. Sub-paragraph's
            // PARA_TEXT records that follow (deeper levels) get harvested
            // into `caption_parts` until SHAPE_COMPONENT ends the caption
            // scope. If this gso turns out to be a non-picture shape, the
            // collected parts are discarded along with `pending_picture`.
            if let Some(p) = pending_picture.as_mut() {
                p.in_caption = true;
            }
        } else if rec.level == child_lvl + 1 && rec.tag == tag::SHAPE_COMPONENT {
            if let Some(p) = pending_picture.as_mut() {
                p.in_caption = false;
                match gso_picture::parse_shape_component_id(&rec.data) {
                    Some(id) if id == gso_picture::GSO_ID_PICTURE => {
                        p.is_pic_confirmed = true;
                    }
                    _ => {
                        // Non-picture shape (line / rect / OLE / …).
                        // The control stays `Unknown` (we don't type
                        // these yet), but if the gso carried a caption
                        // we preserve it on the wrapper so callers can
                        // still locate and edit that text.
                        if !p.caption_parts.is_empty() {
                            para.controls[p.ctrl_idx].caption_text =
                                Some(p.caption_parts.join(" "));
                        }
                        pending_picture = None;
                    }
                }
            }
        } else if rec.tag == tag::PARA_TEXT && rec.level > child_lvl + 1 {
            // Caption paragraph text arrives deeper than the caption's
            // LIST_HEADER. We only collect while `in_caption` is active —
            // PARA_TEXT inside a different sub-structure (e.g. a nested
            // cell) won't match because such nesting is routed through
            // the TABLE / cell branch, not this fallthrough.
            if let Some(p) = pending_picture.as_mut() {
                if p.in_caption {
                    if let Ok(text) = super::paragraph_text::extract_text(&rec) {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            p.caption_parts.push(trimmed.to_string());
                        }
                    }
                }
            }
        } else if rec.level == child_lvl + 2 && rec.tag == tag::SHAPE_COMPONENT_PICTURE {
            // SHAPE_COMPONENT_PICTURE sits one level deeper than its
            // parent SHAPE_COMPONENT (which is at child_lvl + 1) — it's a
            // child of the shape, not a sibling.
            if let Some(p) = pending_picture.take() {
                if p.is_pic_confirmed {
                    if let Some(bin_id) = gso_picture::parse_picture_bin_id(&rec.data) {
                        if !p.caption_parts.is_empty() {
                            para.controls[p.ctrl_idx].caption_text =
                                Some(p.caption_parts.join(" "));
                        }
                        para.controls[p.ctrl_idx].kind =
                            ControlKind::Picture(PictureControl {
                                bin_id,
                                width_hwpu: p.width_hwpu,
                                height_hwpu: p.height_hwpu,
                            });
                    }
                }
            }
        }

        para.raw_records.push(rec);
    }

    Ok(para)
}

/// State carried between the CTRL_HEADER "gso " marker and the
/// SHAPE_COMPONENT / SHAPE_COMPONENT_PICTURE child records that follow it.
struct PendingPicture {
    ctrl_idx: usize,
    width_hwpu: u32,
    height_hwpu: u32,
    is_pic_confirmed: bool,
    /// Toggled on by the caption LIST_HEADER at `child_lvl + 1` and off
    /// again when SHAPE_COMPONENT arrives. While true, any PARA_TEXT at
    /// a deeper level is treated as caption content.
    in_caption: bool,
    caption_parts: Vec<String>,
}

/// Starting at the record after TABLE, read `LIST_HEADER × N` + the cell
/// paragraphs belonging to each. Returns when the next record is no longer
/// a cell at `cell_level`.
///
/// **Observed level convention**: cell PARA_HEADER sits at the same level
/// as its LIST_HEADER (not one level deeper). Its body records (PARA_TEXT /
/// PARA_CHAR_SHAPE / PARA_LINE_SEG) are at `cell_level + 1`.
fn parse_table_cells(
    iter: &mut RecIter,
    tbl: &mut TableControl,
    cell_level: u16,
) -> Result<(), IrError> {
    while let Some(peek) = iter.peek() {
        if peek.tag != tag::LIST_HEADER || peek.level != cell_level {
            break;
        }
        let list_rec = iter.next().unwrap();
        let mut cell = list_header::parse_cell(&list_rec)?;

        while let Some(p) = iter.peek() {
            if p.tag == tag::PARA_HEADER && p.level == cell_level {
                cell.paragraphs.push(parse_paragraph(iter, cell_level)?);
            } else {
                break;
            }
        }
        tbl.cells.push(cell);
    }
    Ok(())
}

/// Extract the `N` suffix from `/BodyText/Section{N}`. Used to order streams
/// numerically (avoids Section2 coming after Section10 in lex order).
pub fn section_index_of(path: &str) -> Option<u32> {
    path.strip_prefix(STREAM_PREFIX).and_then(|s| s.parse().ok())
}

/// Inverse of `parse_section`: flatten a section's record tree into the
/// raw `/BodyText/Section{N}` byte stream, then DEFLATE-compress if the
/// FileHeader's `compressed` bit is set.
///
/// Each paragraph's first record (the PARA_HEADER) is re-emitted from
/// the typed `Paragraph::header` view, so typed mutations of header
/// fields flow into the output automatically. All other records are
/// emitted verbatim from `Paragraph::raw_records` / `Section::raw_records`,
/// which preserves bit-exact fidelity for any record type that doesn't
/// yet have a typed encoder.
///
/// Byte-equality with the original stream on an unmutated section is
/// NOT guaranteed here — that's what `Section::stream_bytes` verbatim
/// pass-through is for. This emitter guarantees *structural* equality:
/// the re-parsed records match the input record sequence 1:1.
pub fn emit_section(section: &Section, compressed: bool) -> Result<Vec<u8>, IrError> {
    let records = collect_section_records(section);
    let raw = record::emit_all(&records);
    if compressed {
        doc_info::compress(&raw)
    } else {
        Ok(raw)
    }
}

/// Flatten section-prelude records + each paragraph's records, with the
/// typed ParagraphHeader pulled back into the first record of each
/// paragraph. Kept separate from `emit_section` so unit tests can
/// inspect the record stream without worrying about DEFLATE.
fn collect_section_records(section: &Section) -> Vec<Record> {
    let mut out: Vec<Record> = section.raw_records.clone();
    for para in &section.paragraphs {
        out.extend(sync_paragraph_records(para));
    }
    out
}

/// Return a paragraph's record list with typed-field updates pulled
/// back into the *first occurrence* of each projected tag, plus the
/// records belonging to every table control interleaved after their
/// TABLE record.
///
/// Typed syncs (first occurrence):
///   * PARA_HEADER     ← `para.header`
///   * PARA_TEXT       ← `para.text` (only when the original raw
///                       record decoded without U+FFFC — see below)
///   * PARA_CHAR_SHAPE ← `para.char_shape_runs`
///   * PARA_LINE_SEG   ← `para.line_segments`
///
/// PARA_TEXT is gated on the *original* raw record: if the raw bytes
/// decode to a text that contains U+FFFC, the paragraph has extended-
/// control payloads (inline pictures / hyperlinks / field markers)
/// that the typed `text` view can't represent. Re-emitting from
/// `para.text` would silently destroy them, so the raw PARA_TEXT
/// stays verbatim and edits to `para.text` are dropped. For pure-text
/// paragraphs, edits flow normally via [`paragraph_text::emit`].
///
/// Table cell interleave: `parse_paragraph` pulls each table's
/// LIST_HEADER + cell paragraph records *out* of the outer paragraph's
/// record stream and stashes them on `ControlKind::Table::cells`.
/// Without the re-injection pass below, the writer's re-encode path
/// would silently drop every cell in every table — a subtle dataloss
/// bug that the prior paragraph-count-only structural test did not
/// catch.
///
/// Only the *first* occurrence of each tag is rewritten. Real HWP5
/// files carry exactly one PARA_* per paragraph, so the policy matches
/// production authoring tools; the first-occurrence rule guards against
/// silent corruption if an unusual fixture ever contains duplicates.
fn sync_paragraph_records(para: &Paragraph) -> Vec<Record> {
    let mut records = para.raw_records.clone();
    let mut header_synced = false;
    let mut text_synced = false;
    let mut char_shape_synced = false;
    let mut line_seg_synced = false;
    for rec in &mut records {
        match rec.tag {
            tag::PARA_HEADER if !header_synced => {
                rec.data = paragraph_header::emit(&para.header);
                header_synced = true;
            }
            tag::PARA_TEXT if !text_synced => {
                // Gate on the *original* record: if the raw PARA_TEXT
                // decodes to a text that already contained U+FFFC, the
                // paragraph has extended-control payloads that the
                // typed view can't represent. Honoring a text edit
                // here would silently destroy those payloads (inline
                // picture markers, hyperlinks, …), so we keep the raw
                // bytes verbatim. Only when the original was pure
                // text do we re-emit from `para.text`.
                let original_lossy = paragraph_text::extract_text(rec)
                    .map(|s| s.contains('\u{FFFC}'))
                    .unwrap_or(true);
                if !original_lossy {
                    if let Ok(bytes) = paragraph_text::emit(&para.text) {
                        rec.data = bytes;
                    }
                    // Emit-side refusal (user inserted U+FFFC or a bare
                    // low-value control): also fall through to verbatim.
                }
                text_synced = true;
            }
            tag::PARA_CHAR_SHAPE if !char_shape_synced => {
                rec.data = paragraph_char_shape::emit(&para.char_shape_runs);
                char_shape_synced = true;
            }
            tag::PARA_LINE_SEG if !line_seg_synced => {
                rec.data = paragraph_line_seg::emit(&para.line_segments);
                line_seg_synced = true;
            }
            _ => {}
        }
    }

    // Interleave each table control's LIST_HEADER + cell paragraph
    // records right after its TABLE record. The nth TABLE record in
    // para.raw_records pairs positionally with the nth Table control
    // in para.controls (parse_paragraph built them in lockstep).
    let mut tables = para.controls.iter().filter_map(|c| match &c.kind {
        ControlKind::Table(t) => Some(t),
        _ => None,
    });
    let mut out: Vec<Record> = Vec::with_capacity(records.len());
    for rec in records {
        let is_table = rec.tag == tag::TABLE;
        let cell_level = rec.level;
        out.push(rec);
        if is_table {
            if let Some(tc) = tables.next() {
                emit_table_cell_records(tc, cell_level, &mut out);
            }
        }
    }
    out
}

/// Walk a table's cells and append `LIST_HEADER(cell) + cell paragraph
/// records` in the order `parse_table_cells` originally read them.
/// Cell paragraphs are themselves run through [`sync_paragraph_records`]
/// so typed mutations inside cells (text / header / char_shape / line
/// seg, and any nested tables) also flow through.
///
/// `cell_level` equals the level of the TABLE record; both the cell's
/// LIST_HEADER and its paragraphs' PARA_HEADER sit at that same level
/// per HWP5's record-tree convention.
fn emit_table_cell_records(tc: &TableControl, cell_level: u16, out: &mut Vec<Record>) {
    for cell in &tc.cells {
        out.push(Record {
            tag: tag::LIST_HEADER,
            level: cell_level,
            data: list_header::emit_cell(cell),
        });
        for para in &cell.paragraphs {
            out.extend(sync_paragraph_records(para));
        }
    }
}
