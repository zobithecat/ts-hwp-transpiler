//! HWPX `Contents/header.xml` → DocInfo-style records.
//!
//! Populates the three fields that drive downstream quality:
//!
//!   * `font_faces` — per-script font tables (hangul / latin / ...),
//!     mirrored from HWP5's `FontFaces` exactly so the HTML preview's
//!     font-family lookup works identically for both containers.
//!   * `border_fills` — cell background colours pulled from
//!     `<hh:borderFill>` + nested `<hc:fillBrush>/<hc:winBrush>`. The
//!     visual role classifier keys off the bg tone, so without these
//!     every HWPX cell gets labelled `value` regardless of the doc's
//!     actual label/value colour convention.
//!   * `char_shapes` — bold / italic / strike / font-size / colour
//!     per `<hh:charPr>`. Surfaces inline formatting to the Markdown
//!     exporter (which already renders `**bold**` etc. through the
//!     existing CharShape path).
//!
//! HWPX declares IDs 1-based while HWP5's typed fields are 0-indexed
//! array slots. We pad a default entry at index 0 so cell-side
//! `borderFillIDRef="N"` can continue to be used as a direct index —
//! same convention both exporters already expect.

use std::io::BufRead;

use hwp_transpiler_core::ir::{
    BorderFill, CharShape, Fill, FontFace, FontFaces, IrError,
};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

#[derive(Debug, Default)]
pub struct HeaderExtract {
    pub font_faces: FontFaces,
    pub border_fills: Vec<BorderFill>,
    pub char_shapes: Vec<CharShape>,
}

pub fn parse_header_xml(xml: &[u8]) -> Result<HeaderExtract, IrError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);

    let mut out = HeaderExtract::default();
    let mut buf = Vec::new();

    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err("header", e))?
        {
            Event::Start(e) => match local_name(&e) {
                "fontfaces" => parse_fontfaces(&mut reader, &mut out.font_faces)?,
                "borderFills" => parse_border_fills(&mut reader, &mut out.border_fills)?,
                "charProperties" => parse_char_properties(&mut reader, &mut out.char_shapes)?,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(out)
}

// ─── Font faces ──────────────────────────────────────────────────────

fn parse_fontfaces<R: BufRead>(
    reader: &mut Reader<R>,
    fonts: &mut FontFaces,
) -> Result<(), IrError> {
    let mut buf = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err("fontfaces", e))?
        {
            Event::End(e) if local_name_bytes(e.name().as_ref()) == "fontfaces" => break,
            Event::Start(e) if local_name(&e) == "fontface" => {
                let lang = string_attr(&e, "lang").unwrap_or_default();
                let slot = match lang.as_str() {
                    "HANGUL" => &mut fonts.hangul,
                    "LATIN" => &mut fonts.latin,
                    "HANJA" => &mut fonts.hanja,
                    "JAPANESE" => &mut fonts.japanese,
                    "SYMBOL" => &mut fonts.symbol,
                    "USER" => &mut fonts.user,
                    _ => &mut fonts.other,
                };
                parse_one_fontface(reader, slot)?;
            }
            Event::Eof => {
                return Err(IrError::Invalid("EOF in fontfaces".into()));
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

fn parse_one_fontface<R: BufRead>(
    reader: &mut Reader<R>,
    slot: &mut Vec<FontFace>,
) -> Result<(), IrError> {
    let mut buf = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err("fontface", e))?
        {
            Event::End(e) if local_name_bytes(e.name().as_ref()) == "fontface" => break,
            Event::Empty(e) if local_name(&e) == "font" => {
                slot.push(FontFace {
                    properties: 0,
                    name: string_attr(&e, "face").unwrap_or_default(),
                    substitute: None,
                    type_info: None,
                    default_name: None,
                });
            }
            Event::Start(e) if local_name(&e) == "font" => {
                // `<hh:font>` with children (typically `<hh:substFont>`).
                let face = string_attr(&e, "face").unwrap_or_default();
                slot.push(FontFace {
                    properties: 0,
                    name: face,
                    substitute: None,
                    type_info: None,
                    default_name: None,
                });
                skip_until_close(reader, e.name().as_ref())?;
            }
            Event::Eof => {
                return Err(IrError::Invalid("EOF in fontface".into()));
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

// ─── Border fills ────────────────────────────────────────────────────

fn parse_border_fills<R: BufRead>(
    reader: &mut Reader<R>,
    fills: &mut Vec<BorderFill>,
) -> Result<(), IrError> {
    // Cells reference fills by their XML-declared ID (1-based). Pad
    // slot 0 with a default so `border_fill_id=N` stays a direct
    // index, matching HWP5's convention.
    if fills.is_empty() {
        fills.push(BorderFill::default());
    }

    let mut buf = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err("borderFills", e))?
        {
            Event::End(e) if local_name_bytes(e.name().as_ref()) == "borderFills" => break,
            Event::Start(e) if local_name(&e) == "borderFill" => {
                let id = u32_attr(&e, "id").unwrap_or(0) as usize;
                let fill = parse_one_border_fill(reader)?;
                while fills.len() <= id {
                    fills.push(BorderFill::default());
                }
                fills[id] = fill;
            }
            Event::Empty(e) if local_name(&e) == "borderFill" => {
                let id = u32_attr(&e, "id").unwrap_or(0) as usize;
                while fills.len() <= id {
                    fills.push(BorderFill::default());
                }
            }
            Event::Eof => {
                return Err(IrError::Invalid("EOF in borderFills".into()));
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

fn parse_one_border_fill<R: BufRead>(
    reader: &mut Reader<R>,
) -> Result<BorderFill, IrError> {
    let mut bf = BorderFill::default();
    let mut buf = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err("borderFill", e))?
        {
            Event::End(e) if local_name_bytes(e.name().as_ref()) == "borderFill" => break,
            Event::Start(e) if local_name(&e) == "fillBrush" => {
                if let Some(rgba) = parse_fill_brush_face_color(reader)? {
                    bf.fill = Fill {
                        kind: Fill::KIND_COLOR,
                        // Mirror HWP5's ColorFill body layout: bytes
                        // 0..=2 are R/G/B so `Fill::back_color()` can
                        // pull the colour without knowing the source
                        // container.
                        body: vec![rgba.0, rgba.1, rgba.2, 0],
                    };
                }
            }
            Event::Eof => {
                return Err(IrError::Invalid("EOF in borderFill".into()));
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(bf)
}

/// Walk a `<hc:fillBrush>` body and pull the first `<hc:winBrush>`'s
/// `faceColor`, decoding `#RRGGBB`. Returns `None` when the brush
/// declares `faceColor="none"` or no winBrush is present — both map
/// to the transparent-fill case.
fn parse_fill_brush_face_color<R: BufRead>(
    reader: &mut Reader<R>,
) -> Result<Option<(u8, u8, u8)>, IrError> {
    let mut out = None;
    let mut buf = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err("fillBrush", e))?
        {
            Event::End(e) if local_name_bytes(e.name().as_ref()) == "fillBrush" => break,
            Event::Empty(e)
                if local_name(&e) == "winBrush"
                    || local_name(&e) == "fillColor" =>
            {
                if let Some(color) = string_attr(&e, "faceColor") {
                    out = parse_hex_color(&color);
                }
            }
            Event::Start(e) => {
                // Unexpected nested start — skip but don't fail.
                skip_until_close(reader, e.name().as_ref())?;
            }
            Event::Eof => {
                return Err(IrError::Invalid("EOF in fillBrush".into()));
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

// ─── Char shapes ─────────────────────────────────────────────────────

fn parse_char_properties<R: BufRead>(
    reader: &mut Reader<R>,
    shapes: &mut Vec<CharShape>,
) -> Result<(), IrError> {
    let mut buf = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err("charProperties", e))?
        {
            Event::End(e) if local_name_bytes(e.name().as_ref()) == "charProperties" => break,
            Event::Start(e) if local_name(&e) == "charPr" => {
                let id = u32_attr(&e, "id").unwrap_or(0) as usize;
                let shape = parse_one_char_pr(reader, &e)?;
                while shapes.len() <= id {
                    shapes.push(CharShape::default());
                }
                shapes[id] = shape;
            }
            Event::Empty(e) if local_name(&e) == "charPr" => {
                let id = u32_attr(&e, "id").unwrap_or(0) as usize;
                let shape = charpr_from_attrs(&e);
                while shapes.len() <= id {
                    shapes.push(CharShape::default());
                }
                shapes[id] = shape;
            }
            Event::Eof => {
                return Err(IrError::Invalid("EOF in charProperties".into()));
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

fn parse_one_char_pr<R: BufRead>(
    reader: &mut Reader<R>,
    start: &BytesStart,
) -> Result<CharShape, IrError> {
    let mut shape = charpr_from_attrs(start);
    let mut buf = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err("charPr", e))?
        {
            Event::End(e) if local_name_bytes(e.name().as_ref()) == "charPr" => break,
            Event::Empty(e) => match local_name(&e) {
                // HWPX always emits `<hh:strikeout/>` and
                // `<hh:underline/>` as children of `<hh:charPr>`, even
                // when neither is applied — the `shape` attribute
                // carries the enable state (`"NONE"` = off, anything
                // else = on). Bold / italic are truly presence-only.
                // Setting strike/underline on every charPr by element
                // presence alone would flip an entire document's
                // worth of text to struck-through or underlined (real
                // bug report: a business-plan HWPX rendered with 100%
                // strikethrough). Check the shape attr before
                // activating the bit.
                "bold" => shape.attr |= 0x0000_0002,
                "italic" => shape.attr |= 0x0000_0001,
                "underline" => {
                    if !shape_attr_is_none(&e) {
                        // bits 2..=4 encode underline kind; "1" is a
                        // simple underline, enough for
                        // `underline_kind() != 0` to return true.
                        shape.attr |= 0x0000_0004;
                        if let Some(c) =
                            string_attr(&e, "color").and_then(|s| parse_hex_color(&s))
                        {
                            shape.underline_color = pack_color(c);
                        }
                    }
                }
                "strikeout" => {
                    if !shape_attr_is_none(&e) {
                        shape.attr |= 1 << 21;
                        if let Some(c) =
                            string_attr(&e, "color").and_then(|s| parse_hex_color(&s))
                        {
                            shape.strike_color = Some(pack_color(c));
                        }
                    }
                }
                "fontRef" => fill_u16_array(&e, &mut shape.font_ids),
                "ratio" => fill_u8_array(&e, &mut shape.ratios, 100),
                "relSz" => fill_u8_array(&e, &mut shape.rel_sizes, 100),
                "spacing" => fill_i8_array(&e, &mut shape.char_spacings),
                "offset" => fill_i8_array(&e, &mut shape.char_offsets),
                _ => {}
            },
            Event::Start(e) => match local_name(&e) {
                "strikeout" => {
                    if !shape_attr_is_none(&e) {
                        shape.attr |= 1 << 21;
                        if let Some(c) =
                            string_attr(&e, "color").and_then(|s| parse_hex_color(&s))
                        {
                            shape.strike_color = Some(pack_color(c));
                        }
                    }
                    skip_until_close(reader, e.name().as_ref())?;
                }
                _ => skip_until_close(reader, e.name().as_ref())?,
            },
            Event::Eof => {
                return Err(IrError::Invalid("EOF in charPr".into()));
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(shape)
}

fn charpr_from_attrs(e: &BytesStart<'_>) -> CharShape {
    let mut shape = CharShape::default();
    if let Some(h) = u32_attr(e, "height") {
        shape.base_size = h as i32;
    } else {
        shape.base_size = 1000;
    }
    if let Some(c) = string_attr(e, "textColor").and_then(|s| parse_hex_color(&s)) {
        shape.color = pack_color(c);
    }
    if let Some(c) = string_attr(e, "shadeColor").and_then(|s| parse_hex_color(&s)) {
        shape.shade_color = pack_color(c);
    }
    if let Some(id) = u32_attr(e, "borderFillIDRef") {
        shape.border_fill_id = Some(id as u16);
    }
    shape
}

/// Slot order for `<hh:fontRef>` / `<hh:ratio>` / etc. Matches HWP5's
/// `FontFaces` ordering so the decoded 7-element arrays interop
/// directly with the HWP5-side records.
const SCRIPT_SLOTS: [&str; 7] = [
    "hangul", "latin", "hanja", "japanese", "other", "symbol", "user",
];

fn fill_u16_array(e: &BytesStart<'_>, out: &mut [u16; 7]) {
    for (i, key) in SCRIPT_SLOTS.iter().enumerate() {
        out[i] = u32_attr(e, key).map(|v| v as u16).unwrap_or(0);
    }
}

fn fill_u8_array(e: &BytesStart<'_>, out: &mut [u8; 7], default: u8) {
    for (i, key) in SCRIPT_SLOTS.iter().enumerate() {
        out[i] = u32_attr(e, key).map(|v| v as u8).unwrap_or(default);
    }
}

fn fill_i8_array(e: &BytesStart<'_>, out: &mut [i8; 7]) {
    for (i, key) in SCRIPT_SLOTS.iter().enumerate() {
        out[i] = i32_attr(e, key)
            .unwrap_or(0)
            .clamp(i8::MIN as i32, i8::MAX as i32) as i8;
    }
}

// ─── Shared helpers ─────────────────────────────────────────────────

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

fn local_name<'a>(e: &'a BytesStart<'_>) -> &'a str {
    local_name_bytes(e.name().into_inner())
}

fn local_name_bytes(bytes: &[u8]) -> &str {
    let start = bytes.iter().position(|&b| b == b':').map(|i| i + 1).unwrap_or(0);
    std::str::from_utf8(&bytes[start..]).unwrap_or("")
}

fn u32_attr(e: &BytesStart<'_>, name: &str) -> Option<u32> {
    string_attr(e, name)?.trim().parse::<u32>().ok()
}

fn i32_attr(e: &BytesStart<'_>, name: &str) -> Option<i32> {
    string_attr(e, name)?.trim().parse::<i32>().ok()
}

fn string_attr(e: &BytesStart<'_>, name: &str) -> Option<String> {
    for attr in e.attributes().flatten() {
        if attr.key.as_ref() == name.as_bytes() {
            return std::str::from_utf8(&attr.value).ok().map(|s| s.to_string());
        }
    }
    None
}

/// True when the element carries `shape="NONE"` — HWPX's way of
/// signalling "this style hook is declared but not applied". Any
/// other value (including the absence of the attribute) is treated
/// as active; most documents emit e.g. `shape="SOLID"` for real
/// strike / underline and `shape="NONE"` for the default.
fn shape_attr_is_none(e: &BytesStart<'_>) -> bool {
    string_attr(e, "shape")
        .map(|s| s.eq_ignore_ascii_case("NONE"))
        .unwrap_or(false)
}

/// Decode `#RRGGBB` (or `none`) into an (R, G, B) triple.
fn parse_hex_color(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.trim();
    if !s.starts_with('#') || s.len() != 7 {
        return None;
    }
    let r = u8::from_str_radix(&s[1..3], 16).ok()?;
    let g = u8::from_str_radix(&s[3..5], 16).ok()?;
    let b = u8::from_str_radix(&s[5..7], 16).ok()?;
    Some((r, g, b))
}

/// Pack an (R, G, B) triple into HWP5's u32 colour format (R in the
/// low byte) so downstream accessors that shift-and-mask work the
/// same way for both containers.
fn pack_color((r, g, b): (u8, u8, u8)) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

fn xml_err(context: &str, e: quick_xml::Error) -> IrError {
    IrError::Invalid(format!("hwpx header ({context}): {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_font_face_names_per_lang() {
        let xml = r##"<hh:head xmlns:hh="h">
            <hh:refList>
              <hh:fontfaces itemCnt="2">
                <hh:fontface lang="HANGUL" fontCnt="2">
                  <hh:font id="0" face="굴림"/>
                  <hh:font id="1" face="맑은 고딕"/>
                </hh:fontface>
                <hh:fontface lang="LATIN" fontCnt="1">
                  <hh:font id="0" face="Arial"/>
                </hh:fontface>
              </hh:fontfaces>
            </hh:refList>
          </hh:head>"##;
        let h = parse_header_xml(xml.as_bytes()).expect("parse");
        assert_eq!(h.font_faces.hangul.len(), 2);
        assert_eq!(h.font_faces.hangul[0].name, "굴림");
        assert_eq!(h.font_faces.hangul[1].name, "맑은 고딕");
        assert_eq!(h.font_faces.latin.len(), 1);
        assert_eq!(h.font_faces.latin[0].name, "Arial");
    }

    #[test]
    fn border_fill_with_color_populates_fill() {
        let xml = r##"<hh:head xmlns:hh="h" xmlns:hc="c">
            <hh:refList>
              <hh:borderFills itemCnt="1">
                <hh:borderFill id="2">
                  <hc:fillBrush>
                    <hc:winBrush faceColor="#CCDDEE" hatchColor="#999999" alpha="0"/>
                  </hc:fillBrush>
                </hh:borderFill>
              </hh:borderFills>
            </hh:refList>
          </hh:head>"##;
        let h = parse_header_xml(xml.as_bytes()).expect("parse");
        // slot 0 is default, slot 1 never declared (pad), slot 2 is ours.
        assert!(h.border_fills.len() >= 3);
        let (r, g, b, a) = h.border_fills[2].fill.back_color().expect("color");
        assert_eq!((r, g, b), (0xCC, 0xDD, 0xEE));
        assert_eq!(a, 0xFF);
    }

    #[test]
    fn border_fill_with_none_face_stays_transparent() {
        let xml = r##"<hh:head xmlns:hh="h" xmlns:hc="c">
            <hh:refList>
              <hh:borderFills itemCnt="1">
                <hh:borderFill id="1">
                  <hc:fillBrush>
                    <hc:winBrush faceColor="none" hatchColor="#999999" alpha="0"/>
                  </hc:fillBrush>
                </hh:borderFill>
              </hh:borderFills>
            </hh:refList>
          </hh:head>"##;
        let h = parse_header_xml(xml.as_bytes()).expect("parse");
        assert!(h.border_fills[1].fill.back_color().is_none());
    }

    #[test]
    fn char_pr_bold_italic_strike_decoded() {
        let xml = r##"<hh:head xmlns:hh="h" xmlns:hc="c">
            <hh:refList>
              <hh:charProperties itemCnt="1">
                <hh:charPr id="0" height="1100" textColor="#112233">
                  <hh:fontRef hangul="0" latin="0" hanja="0" japanese="0" other="0" symbol="0" user="0"/>
                  <hh:bold/>
                  <hh:italic/>
                  <hh:strikeout shape="SOLID" color="#FF0000"/>
                </hh:charPr>
              </hh:charProperties>
            </hh:refList>
          </hh:head>"##;
        let h = parse_header_xml(xml.as_bytes()).expect("parse");
        let s = &h.char_shapes[0];
        assert_eq!(s.base_size, 1100);
        assert_eq!(s.color, 0x0033_2211);
        assert!(s.bold(), "bold child should set bit 1");
        assert!(s.italic(), "italic child should set bit 0");
        assert!(s.strike(), "strikeout child should set bit 21");
        assert_eq!(s.strike_color, Some(0x0000_00FF));
    }

    #[test]
    fn char_pr_without_style_children_has_none_set() {
        let xml = r##"<hh:head xmlns:hh="h">
            <hh:refList>
              <hh:charProperties itemCnt="1">
                <hh:charPr id="0" height="1000" textColor="#000000">
                  <hh:fontRef hangul="0" latin="0" hanja="0" japanese="0" other="0" symbol="0" user="0"/>
                </hh:charPr>
              </hh:charProperties>
            </hh:refList>
          </hh:head>"##;
        let h = parse_header_xml(xml.as_bytes()).expect("parse");
        let s = &h.char_shapes[0];
        assert!(!s.bold());
        assert!(!s.italic());
        assert!(!s.strike());
    }

    #[test]
    fn strikeout_with_shape_none_does_not_flip_strike_bit() {
        // Regression: HWPX always emits `<hh:strikeout/>` inside
        // `<hh:charPr>`, with `shape="NONE"` as the "not applied"
        // sentinel. A docset caught the original bug when every run
        // inherited strike from a default charPr that carried
        // `<hh:strikeout shape="NONE"/>`.
        let xml = r##"<hh:head xmlns:hh="h">
            <hh:refList>
              <hh:charProperties itemCnt="2">
                <hh:charPr id="0" height="1000" textColor="#000000">
                  <hh:fontRef hangul="0" latin="0" hanja="0" japanese="0" other="0" symbol="0" user="0"/>
                  <hh:strikeout shape="NONE" color="#000000"/>
                  <hh:underline shape="NONE" color="#000000"/>
                </hh:charPr>
                <hh:charPr id="1" height="1000" textColor="#000000">
                  <hh:fontRef hangul="0" latin="0" hanja="0" japanese="0" other="0" symbol="0" user="0"/>
                  <hh:strikeout shape="SOLID" color="#FF0000"/>
                  <hh:underline shape="SOLID" color="#00FF00"/>
                </hh:charPr>
              </hh:charProperties>
            </hh:refList>
          </hh:head>"##;
        let h = parse_header_xml(xml.as_bytes()).expect("parse");
        assert!(!h.char_shapes[0].strike(), "shape=NONE should not set strike");
        assert_eq!(h.char_shapes[0].underline_kind(), 0);
        assert!(h.char_shapes[1].strike(), "shape=SOLID should set strike");
        assert_ne!(h.char_shapes[1].underline_kind(), 0);
    }
}
