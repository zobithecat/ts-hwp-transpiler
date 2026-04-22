//! BodyText IR — sections → paragraphs → (text, char-shape runs, line segments,
//! controls). Controls embed tables, equations, pictures, shapes, footnotes,
//! and anything else HWP treats as an inline object.

use serde::{Deserialize, Serialize};

use super::doc_info::Record;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Section {
    pub properties: SectionProperties,
    pub paragraphs: Vec<Paragraph>,
    /// Records that belong to the section as a whole (e.g. LIST_HEADER /
    /// PAGE_DEF / FOOTNOTE_SHAPE preceding the first paragraph), kept
    /// verbatim until typed encoders promote them out.
    pub raw_records: Vec<Record>,
    /// Exact stream bytes as read (raw DEFLATE when FileHeader.compressed
    /// is set). Verbatim-if-unchanged write path; cleared by mutation.
    pub stream_bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SectionProperties {
    pub page_width_hwpu: u32,
    pub page_height_hwpu: u32,
    /// `[top, right, bottom, left]` margins in HWPUNIT (1 pt = 100 HWPUNIT).
    pub margins_hwpu: [u32; 4],
    pub columns: u16,
}

/// HWP5 ParagraphHeader (tag 0x0042). The section stream is a flat record
/// sequence where each PARA_HEADER starts a paragraph; records that follow
/// (up to the next PARA_HEADER) carry text, shape runs, line segments, and
/// inline controls for that paragraph.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ParagraphHeader {
    /// Raw `char_count` field. Top bit is a marker (use `real_char_count`
    /// for the UTF-16 code-unit count).
    pub char_count: u32,
    /// Bitfield indicating which optional control records follow.
    pub control_mask: u32,
    pub para_shape_id: u16,
    pub style_id: u8,
    /// Column/page/section break kind (0 = none).
    pub divide_sort: i8,
    pub char_shape_count: u16,
    pub range_tag_count: u16,
    pub line_align_count: u16,
    pub instance_id: u32,
    /// Present in HWP 5.0.3.2+.
    pub change_tracking_merged: Option<u16>,
}

impl ParagraphHeader {
    /// Actual UTF-16 code-unit count, stripping the top-bit marker that
    /// HWP5 uses for flags like "last paragraph in stream".
    pub fn real_char_count(&self) -> u32 {
        self.char_count & 0x7FFF_FFFF
    }
    /// Whether the top-bit marker is set. Exact semantics vary across HWP5
    /// revisions; preserved so the writer can round-trip the flag.
    pub fn has_marker_bit(&self) -> bool {
        self.char_count & 0x8000_0000 != 0
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Paragraph {
    pub header: ParagraphHeader,
    /// Decoded text. Extended 16-byte control markers are replaced with
    /// U+FFFC (Object Replacement Character); inline controls (TAB, NBSP,
    /// LINE_BREAK, ...) are converted to their Unicode equivalents.
    pub text: String,
    pub char_shape_runs: Vec<CharShapeRun>,
    pub line_segments: Vec<LineSegment>,
    pub controls: Vec<Control>,
    /// All non-header records belonging to this paragraph, held verbatim
    /// until their typed encoders land.
    pub raw_records: Vec<Record>,
}

/// PARA_CHAR_SHAPE (tag 0x0044) entry. Each paragraph has N runs; each run
/// applies `char_shape_id` from `start` (UTF-16 code-unit offset) until the
/// next run (or end of paragraph).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharShapeRun {
    pub start: u32,
    /// Stored as u32 in PARA_CHAR_SHAPE, resolved via `doc.doc_info.char_shapes[id]`.
    pub char_shape_id: u32,
}

/// PARA_LINE_SEG (tag 0x0045) entry — one per line as laid out at save time.
/// Fields are HWPUNIT (1 pt = 100 HWPUNIT).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LineSegment {
    /// UTF-16 code-unit offset of the first char on this line.
    pub text_start: u32,
    pub vertical_position_hwpu: i32,
    pub line_height_hwpu: i32,
    pub text_height_hwpu: i32,
    pub baseline_distance_hwpu: i32,
    pub line_spacing_hwpu: i32,
    pub start_x_hwpu: i32,
    pub width_hwpu: i32,
    /// Flag bits (alignment, first-line etc.).
    pub tag: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Control {
    pub kind: ControlKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ControlKind {
    Table(TableControl),
    Equation(EquationControl),
    Picture(PictureControl),
    Unknown { code: [u8; 4], data: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TableControl {
    /// Flags: page_break behaviour, repeat_header, ...
    pub attribute: u32,
    pub rows: u16,
    pub cols: u16,
    pub cell_spacing: i16,
    /// `[left, right, top, bottom]` inner padding in HWPUNIT.
    pub padding: [i16; 4],
    /// Cell count per row (HWP tables can have ragged rows when merged).
    pub row_cell_counts: Vec<u16>,
    pub border_fill_id: u16,
    /// HWP 5.0.1.0+. Each zone is a rectangular region within the table
    /// overridden by a different BorderFill.
    pub valid_zones: Vec<ValidZone>,
    pub cells: Vec<TableCell>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ValidZone {
    pub start_col: u16,
    pub start_row: u16,
    pub end_col: u16,
    pub end_row: u16,
    pub border_fill_id: u16,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TableCell {
    /// Paragraph count for this cell's `ParagraphList`. Stored as `i32` in
    /// HWP5 (sInt4); kept here so `paragraphs.len()` and the on-disk count
    /// can be reconciled if/when the cell is mutated.
    pub para_count: i32,
    /// `ListHeaderProperty` bitfield (text direction, line break behaviour,
    /// vertical alignment, ...). Held opaque until typed.
    pub property: u32,
    pub col: u16,
    pub row: u16,
    pub col_span: u16,
    pub row_span: u16,
    pub width_hwpu: u32,
    pub height_hwpu: u32,
    /// `[left, right, top, bottom]` inner padding (HWPUNIT).
    pub padding_hwpu: [u16; 4],
    pub border_fill_id: u16,
    /// Text-region width (HWPUNIT) — `width - leftMargin - rightMargin`.
    /// Stored explicitly because hwplib's writer always emits it.
    pub text_width_hwpu: u32,
    pub paragraphs: Vec<Paragraph>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EquationControl {
    pub script: String,
    pub font: Option<String>,
    pub size_hwpu: i32,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PictureControl {
    pub bin_id: String,
    pub width_hwpu: u32,
    pub height_hwpu: u32,
}
