//! DocInfo stream IR. Mirrors the HWP5 DocInfo record tree. Typed fields
//! (`properties`, `font_faces`, ...) are populated progressively; until a
//! tag's typed encoder lands, the record lives in `raw_records` verbatim.

use serde::{Deserialize, Serialize};

/// HWP5 record: a tag + nesting level + opaque payload. Shared between the
/// DocInfo and BodyText streams (both use the same TLV framing).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub tag: u16,
    pub level: u16,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DocInfo {
    pub properties: DocProperties,
    pub font_faces: FontFaces,
    pub border_fills: Vec<BorderFill>,
    pub char_shapes: Vec<CharShape>,
    pub para_shapes: Vec<ParaShape>,
    pub styles: Vec<Style>,
    pub bin_data: Vec<BinData>,
    pub unknown_records: Vec<UnknownRecord>,

    /// Parsed record stream (primary source of truth until typed encoders
    /// migrate individual records into the structured fields above).
    pub raw_records: Vec<Record>,
    /// Exact stream bytes as read (possibly compressed). Present → writer
    /// emits verbatim. Cleared by [`DocInfo::records_mut`] to force
    /// re-encode when the caller mutates records.
    pub stream_bytes: Option<Vec<u8>>,
}

impl DocInfo {
    /// Mutable access to records. Clears `stream_bytes` so the writer
    /// re-encodes from the current record state instead of pass-through.
    pub fn records_mut(&mut self) -> &mut Vec<Record> {
        self.stream_bytes = None;
        &mut self.raw_records
    }

    /// Explicit invalidation of the verbatim cache. Call this if you mutate
    /// typed fields (char_shapes, styles, ...) directly — those paths don't
    /// go through `records_mut`.
    pub fn invalidate_verbatim(&mut self) {
        self.stream_bytes = None;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DocProperties {
    pub section_count: u16,
    pub page_start_number: u16,
    pub footnote_start_number: u16,
    pub endnote_start_number: u16,
    pub picture_start_number: u16,
    pub table_start_number: u16,
    pub equation_start_number: u16,
    pub total_character_count: u32,
    pub total_page_count: u32,
    /// Post-core payload. HWP5 revisions include different trailing fields
    /// here (caret position, edit session info, ...). Kept as bytes to
    /// preserve lossless round-trip; `caret_position()` decodes the common
    /// 16-byte `[u32; 4]` form when present.
    pub tail: Vec<u8>,
}

impl DocProperties {
    /// Decode `tail[..16]` as `[section, list, paragraph, char]` caret
    /// position if enough bytes are present. Modern blank docs ship only a
    /// 4-byte tail (single u32) — returns `None` in that case.
    pub fn caret_position(&self) -> Option<[u32; 4]> {
        if self.tail.len() < 16 {
            return None;
        }
        let mut out = [0u32; 4];
        for (i, v) in out.iter_mut().enumerate() {
            *v = u32::from_le_bytes([
                self.tail[i * 4],
                self.tail[i * 4 + 1],
                self.tail[i * 4 + 2],
                self.tail[i * 4 + 3],
            ]);
        }
        Some(out)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FontFaces {
    pub hangul: Vec<FontFace>,
    pub latin: Vec<FontFace>,
    pub hanja: Vec<FontFace>,
    pub japanese: Vec<FontFace>,
    pub other: Vec<FontFace>,
    pub symbol: Vec<FontFace>,
    pub user: Vec<FontFace>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontFace {
    /// Flag byte controlling which optional fields follow in the binary.
    /// bit 7 (0x80) → has_substitute, bit 6 (0x40) → has_type_info,
    /// bit 5 (0x20) → has_default_name.
    pub properties: u8,
    pub name: String,
    pub substitute: Option<SubstituteFont>,
    pub type_info: Option<FontTypeInfo>,
    pub default_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubstituteFont {
    pub kind: u8,
    pub name: String,
}

/// Panose 1.0 font classification (10 bytes). Raw values; interpretation is
/// a rendering-layer concern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontTypeInfo {
    pub family: u8,
    pub serif: u8,
    pub weight: u8,
    pub proportion: u8,
    pub contrast: u8,
    pub stroke_variation: u8,
    pub arm_style: u8,
    pub letterform: u8,
    pub midline: u8,
    pub x_height: u8,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BorderFill {
    /// Bit flags (3D, shadow, diagonal shape / rotation, center line).
    pub attribute: u16,
    /// Order: `[left, right, top, bottom]` — matches hwplib `BorderFillReader`.
    pub borders: [Border; 4],
    pub diagonal: Border,
    pub fill: Fill,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Border {
    pub kind: u8,
    pub width: u8,
    /// Packed RGBA. Low byte = R, high byte = A.
    pub color: u32,
}

/// Fill descriptor. `kind` is the raw HWP5 bitfield: bit 0 = Color, bit 1 =
/// Gradation, bit 2 = Image. `body` is the fill-body payload (lazily decoded
/// in later rounds into typed Color/Gradation/Image variants).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fill {
    pub kind: u32,
    pub body: Vec<u8>,
}

impl Fill {
    pub const KIND_COLOR: u32 = 0x01;
    pub const KIND_GRADATION: u32 = 0x02;
    pub const KIND_IMAGE: u32 = 0x04;

    pub fn is_color(&self) -> bool { self.kind & Self::KIND_COLOR != 0 }
    pub fn is_gradation(&self) -> bool { self.kind & Self::KIND_GRADATION != 0 }
    pub fn is_image(&self) -> bool { self.kind & Self::KIND_IMAGE != 0 }

    /// Decode the back-colour of a solid-color fill. HWP5's `ColorFill`
    /// body starts with a u32 `backColor`: byte 0 = R, byte 1 = G,
    /// byte 2 = B. Byte 3 is **not** a straight alpha channel — TRL
    /// fixture has solid colours like `#E5E5E5` stored with byte[3]=0,
    /// and the alpha-like bit there is a reserved/pattern flag rather
    /// than transparency. A solid `KIND_COLOR` fill IS always opaque;
    /// transparency is modelled by the kind bits (`is_color()` = false
    /// for no fill). So we hard-set `a = 0xFF` for any colour fill to
    /// match the observed on-screen semantic.
    ///
    /// Returns `None` when the fill isn't `KIND_COLOR` or the body is
    /// too short.
    pub fn back_color(&self) -> Option<(u8, u8, u8, u8)> {
        if !self.is_color() || self.body.len() < 3 {
            return None;
        }
        Some((self.body[0], self.body[1], self.body[2], 0xFF))
    }
}

/// HWP5 CharShape record.
///
/// The 7-element arrays are indexed by script slot (hangul / latin / hanja /
/// japanese / other / symbol / user), matching `FontFaces` order.
///
/// `attr` is kept as raw `u32` to preserve all bits losslessly across
/// round-trip. Use the accessor methods for common flags; multi-bit fields
/// like `underline_kind` can be extracted with explicit shifts.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CharShape {
    pub font_ids: [u16; 7],
    pub ratios: [u8; 7],
    pub char_spacings: [i8; 7],
    pub rel_sizes: [u8; 7],
    pub char_offsets: [i8; 7],
    /// Font size in 1/100 pt (1000 = 10 pt).
    pub base_size: i32,
    /// Raw attribute bitfield. See accessor methods for decoded views.
    pub attr: u32,
    pub shadow_offset_x: i8,
    pub shadow_offset_y: i8,
    pub color: u32,
    pub underline_color: u32,
    pub shade_color: u32,
    pub shadow_color: u32,
    pub border_fill_id: Option<u16>,
    pub strike_color: Option<u32>,
}

impl CharShape {
    pub fn italic(&self) -> bool { self.attr & 0x0000_0001 != 0 }
    pub fn bold(&self) -> bool { self.attr & 0x0000_0002 != 0 }
    pub fn underline_kind(&self) -> u8 { ((self.attr >> 2) & 0x07) as u8 }
    pub fn underline_shape(&self) -> u8 { ((self.attr >> 5) & 0x07) as u8 }
    pub fn outline_kind(&self) -> u8 { ((self.attr >> 8) & 0x07) as u8 }
    pub fn shadow_kind(&self) -> u8 { ((self.attr >> 11) & 0x0F) as u8 }
    pub fn emboss(&self) -> bool { self.attr & (1 << 15) != 0 }
    pub fn engrave(&self) -> bool { self.attr & (1 << 16) != 0 }
    /// 0 = none, 1 = superscript, 2 = subscript (2-bit field, bits 17..=18).
    pub fn script(&self) -> u8 { ((self.attr >> 17) & 0x03) as u8 }
    pub fn is_superscript(&self) -> bool { self.script() == 1 }
    pub fn is_subscript(&self) -> bool { self.script() == 2 }
    pub fn strike(&self) -> bool { self.attr & (1 << 21) != 0 }
}

/// HWP5 ParaShape record. Carries alignment, margins, line spacing, border
/// geometry, and version-dependent extras (attribute2/3 + the 5.0.3.0 line
/// spacing pair). `attribute` is raw bits to preserve lossless round-trip;
/// accessors decode common fields.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ParaShape {
    /// Raw bitfield. Bits: align (0..=2), break_latin (3..=4), break_non_latin
    /// (5), snap_to_grid (6), condense (7..=14), widow_orphan (15),
    /// keep_with_next (16), keep_lines (17), page_break_before (18),
    /// vertical_align (19..=20), font_line_height (21), heading_kind (22..=23),
    /// heading_level (24..=26), border_connect (27), border_ignore_margin (28).
    pub attribute: u32,
    pub left_margin: i32,
    pub right_margin: i32,
    pub indent: i32,
    pub top_space: i32,
    pub bottom_space: i32,
    /// Legacy line spacing field. In HWP 5.0.3.0+ semantics moved to the
    /// optional `line_spacing_kind`/`line_spacing` pair below; this byte
    /// range is still present for compatibility.
    pub line_space_legacy: i32,
    pub tab_def_id: u16,
    pub heading_id: u16,
    pub border_fill_id: u16,
    pub left_border_space: i16,
    pub right_border_space: i16,
    pub top_border_space: i16,
    pub bottom_border_space: i16,
    /// HWP 5.0.1.7+.
    pub attribute2: Option<u32>,
    /// HWP 5.0.2.5+.
    pub attribute3: Option<u32>,
    /// HWP 5.0.3.0+. Supersedes `line_space_legacy`.
    pub line_spacing_kind: Option<u32>,
    pub line_spacing: Option<u32>,
}

impl ParaShape {
    /// 0 = Justify, 1 = Left, 2 = Right, 3 = Center, 4 = Distribute,
    /// 5 = DistributeSpace.
    pub fn align(&self) -> u8 { (self.attribute & 0x07) as u8 }
    pub fn snap_to_grid(&self) -> bool { self.attribute & (1 << 6) != 0 }
    pub fn keep_with_next(&self) -> bool { self.attribute & (1 << 16) != 0 }
    pub fn keep_lines(&self) -> bool { self.attribute & (1 << 17) != 0 }
    pub fn page_break_before(&self) -> bool { self.attribute & (1 << 18) != 0 }
    pub fn heading_level(&self) -> u8 { ((self.attribute >> 24) & 0x07) as u8 }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Style {
    pub name: String,
    pub english_name: String,
    /// Bit 0..=2: style_type (0 = paragraph, 1 = character). Other bits
    /// reserved in HWP5.
    pub properties: u8,
    pub next_style_id: u8,
    pub lang_id: i16,
    pub para_shape_id: u16,
    pub char_shape_id: u16,
}

/// HWP5 BinData record (DocInfo tag 0x0012). Describes one entry in the
/// document's binary-content table — typically an image, but also OLE
/// objects and external file links.
///
/// `property` is the raw u16 bitfield (low nibble = type, next 4 bits =
/// compression hint, next 4 = status, top 4 = reserved). The `Option`
/// fields are populated only for the type they apply to:
///
///   - Link        → `absolute_path` + `relative_path`
///   - Embedding   → `bin_data_id` (matches `/BinData/BIN<N>` storage
///                                  stream) + `extension`
///   - Storage     → `bin_data_id` + `extension`
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinData {
    pub property: u16,
    pub absolute_path: Option<String>,
    pub relative_path: Option<String>,
    pub bin_data_id: Option<u16>,
    pub extension: Option<String>,
}

impl BinData {
    pub const TYPE_LINK: u16 = 0;
    pub const TYPE_EMBEDDING: u16 = 1;
    pub const TYPE_STORAGE: u16 = 2;

    pub fn type_kind(&self) -> u16 {
        self.property & 0x000F
    }
    pub fn is_link(&self) -> bool {
        self.type_kind() == Self::TYPE_LINK
    }
    pub fn is_embedding(&self) -> bool {
        self.type_kind() == Self::TYPE_EMBEDDING
    }
    pub fn is_storage(&self) -> bool {
        self.type_kind() == Self::TYPE_STORAGE
    }
}

/// Tagged record whose decoder isn't implemented yet. Held verbatim by
/// typed-field DocInfo once we start promoting known tags out of
/// `raw_records`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnknownRecord {
    pub tag: u16,
    pub level: u16,
    pub data: Vec<u8>,
}
