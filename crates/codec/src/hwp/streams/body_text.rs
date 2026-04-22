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
        } else if rec.level == child_lvl + 1 && rec.tag == tag::SHAPE_COMPONENT {
            if let Some(p) = pending_picture.as_mut() {
                match gso_picture::parse_shape_component_id(&rec.data) {
                    Some(id) if id == gso_picture::GSO_ID_PICTURE => {
                        p.is_pic_confirmed = true;
                    }
                    _ => {
                        // Some other shape (line / rect / OLE / …); leave
                        // the control as Unknown and stop tracking.
                        pending_picture = None;
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
