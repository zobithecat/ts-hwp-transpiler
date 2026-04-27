//! Surgical attribute-level rewriter for `Contents/header.xml`.
//!
//! The HWPX writer used to ship `header.xml` straight from
//! `unknown_streams` so any DocInfo-side mutation (font names, char
//! shape colours, paragraph alignment, border fill colour) silently
//! lost on write. Regenerating the whole file from IR isn't safe
//! either — `parse_header_xml` only decodes a subset
//! (fontfaces / charProperties / paraProperties / borderFills), so a
//! from-scratch emit would drop styles, numberings, lineSpacing,
//! borders, kerning flags, the doc's typeInfo Panose data, etc.
//!
//! Instead, we walk the original XML byte-by-byte and overlay only
//! the attributes the IR can express. Every other byte (including
//! comments, whitespace, namespace declarations, sections we don't
//! parse) flows through verbatim. The result is: unmutated docs come
//! out semantically identical (byte-format may shift in the rewritten
//! Start tags but viewers don't care); mutated docs reflect the IR
//! change in exactly the attribute that moved.
//!
//! Supported overlays (Phase 1):
//!   * `<hh:align horizontal=…>` inside `<hh:paraPr>` — from
//!     `ParaShape::align()`.
//!   * `<hh:charPr height=… textColor=… shadeColor=… borderFillIDRef=…>`
//!     — from `CharShape` parent attrs (only attrs already present on
//!     the element are touched).
//!   * `<hh:strikeout shape=… color=…>` and `<hh:underline shape=… color=…>`
//!     inside `<hh:charPr>` — from `CharShape::strike()`/
//!     `underline_kind()` and the matching colour fields.
//!   * `<hh:font face=…>` inside `<hh:fontface>` — from
//!     `FontFace::name`.
//!   * `<hc:winBrush faceColor=…>` inside `<hh:borderFill>` — from
//!     the IR's solid colour fill (gradation / image fills are left
//!     alone).
//!
//! Not yet handled — bold / italic toggling (presence-only children
//! that need structural insert before `</hh:charPr>`), full add /
//! remove of whole shapes, multi-script CharShape arrays
//! (`<hh:fontRef>` / `<hh:ratio>` / …), gradation / image fill
//! mutation. The unmutated path round-trips these correctly because
//! the rewriter never touches their bytes.

use std::collections::HashMap;

use hwp_transpiler_core::ir::{CharShape, Fill, IrDocument, IrError};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

pub fn rewrite_header_xml(original: &[u8], doc: &IrDocument) -> Result<Vec<u8>, IrError> {
    let mut reader = Reader::from_reader(original);
    reader.config_mut().trim_text(false);

    let mut out: Vec<u8> = Vec::with_capacity(original.len());
    let mut cursor: usize = 0;

    let mut para_pr_id: Option<u32> = None;
    let mut char_pr_id: Option<u32> = None;
    let mut fontface_slot: Option<&'static str> = None;
    let mut fontface_index: usize = 0;
    let mut border_fill_id: Option<u32> = None;

    let mut buf = Vec::new();

    loop {
        let event_start = reader.buffer_position() as usize;
        let evt = reader
            .read_event_into(&mut buf)
            .map_err(|e| IrError::Invalid(format!("hwpx header rewrite: {e}")))?;
        let event_end = reader.buffer_position() as usize;

        match evt {
            Event::Start(ref e) => {
                let name = local_name(e);
                match name {
                    "paraPr" => {
                        para_pr_id = u32_attr(e, "id");
                    }
                    "charPr" => {
                        char_pr_id = u32_attr(e, "id");
                        if let Some(id) = char_pr_id {
                            if let Some(shape) = doc.doc_info.char_shapes.get(id as usize) {
                                let overrides = charpr_attr_overrides(shape);
                                replace_with_overlay(
                                    original, &mut out, &mut cursor,
                                    event_start, event_end,
                                    e, false, &overrides,
                                )?;
                            }
                        }
                    }
                    "fontface" => {
                        fontface_slot = lang_to_slot(string_attr(e, "lang").as_deref());
                        fontface_index = 0;
                    }
                    "font" => {
                        rewrite_font_event(
                            doc, original, &mut out, &mut cursor,
                            event_start, event_end,
                            e, false,
                            fontface_slot, &mut fontface_index,
                        )?;
                    }
                    "borderFill" => {
                        border_fill_id = u32_attr(e, "id");
                    }
                    "strikeout" if char_pr_id.is_some() => {
                        rewrite_strikeout(
                            doc, original, &mut out, &mut cursor,
                            event_start, event_end,
                            e, false, char_pr_id.unwrap(),
                        )?;
                    }
                    "underline" if char_pr_id.is_some() => {
                        rewrite_underline(
                            doc, original, &mut out, &mut cursor,
                            event_start, event_end,
                            e, false, char_pr_id.unwrap(),
                        )?;
                    }
                    _ => {}
                }
            }
            Event::Empty(ref e) => {
                let name = local_name(e);
                match name {
                    "align" if para_pr_id.is_some() => {
                        let id = para_pr_id.unwrap();
                        if let Some(shape) =
                            doc.doc_info.para_shapes.get(id as usize)
                        {
                            let mut overrides = HashMap::new();
                            overrides.insert(
                                "horizontal".to_string(),
                                align_to_hwpx(shape.align()).to_string(),
                            );
                            replace_with_overlay(
                                original, &mut out, &mut cursor,
                                event_start, event_end,
                                e, true, &overrides,
                            )?;
                        }
                    }
                    "charPr" => {
                        // Empty charPr (no children). Still overlay
                        // its parent attrs from IR.
                        let id = u32_attr(e, "id");
                        if let Some(id) = id {
                            if let Some(shape) =
                                doc.doc_info.char_shapes.get(id as usize)
                            {
                                let overrides = charpr_attr_overrides(shape);
                                replace_with_overlay(
                                    original, &mut out, &mut cursor,
                                    event_start, event_end,
                                    e, true, &overrides,
                                )?;
                            }
                        }
                    }
                    "font" => {
                        rewrite_font_event(
                            doc, original, &mut out, &mut cursor,
                            event_start, event_end,
                            e, true,
                            fontface_slot, &mut fontface_index,
                        )?;
                    }
                    "fontface" => {
                        // Self-closing fontface — no children, no
                        // fonts to rewrite. Reset the slot so a
                        // following sibling fontface starts fresh.
                        fontface_slot = None;
                    }
                    "winBrush" if border_fill_id.is_some() => {
                        let id = border_fill_id.unwrap();
                        if let Some(bf) =
                            doc.doc_info.border_fills.get(id as usize)
                        {
                            // Only mutate solid-colour fills — the IR
                            // doesn't reflect gradation / image fills,
                            // so we can't safely overlay them.
                            if (bf.fill.kind & Fill::KIND_COLOR) != 0 {
                                if let Some((r, g, b, _)) = bf.fill.back_color() {
                                    let mut overrides = HashMap::new();
                                    overrides.insert(
                                        "faceColor".to_string(),
                                        format!("#{:02X}{:02X}{:02X}", r, g, b),
                                    );
                                    replace_with_overlay(
                                        original, &mut out, &mut cursor,
                                        event_start, event_end,
                                        e, true, &overrides,
                                    )?;
                                }
                            }
                        }
                    }
                    "strikeout" if char_pr_id.is_some() => {
                        rewrite_strikeout(
                            doc, original, &mut out, &mut cursor,
                            event_start, event_end,
                            e, true, char_pr_id.unwrap(),
                        )?;
                    }
                    "underline" if char_pr_id.is_some() => {
                        rewrite_underline(
                            doc, original, &mut out, &mut cursor,
                            event_start, event_end,
                            e, true, char_pr_id.unwrap(),
                        )?;
                    }
                    _ => {}
                }
            }
            Event::End(ref e) => {
                let end_name = e.name();
                let name_bytes = end_name.as_ref();
                let name = local_name_from_bytes(name_bytes);
                match name {
                    "paraPr" => para_pr_id = None,
                    "charPr" => char_pr_id = None,
                    "fontface" => fontface_slot = None,
                    "borderFill" => border_fill_id = None,
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }

        buf.clear();
    }

    out.extend_from_slice(&original[cursor..]);
    Ok(out)
}

// ─── Per-element overlay helpers ────────────────────────────────────

fn rewrite_font_event(
    doc: &IrDocument,
    original: &[u8],
    out: &mut Vec<u8>,
    cursor: &mut usize,
    event_start: usize,
    event_end: usize,
    e: &BytesStart<'_>,
    is_empty: bool,
    fontface_slot: Option<&'static str>,
    fontface_index: &mut usize,
) -> Result<(), IrError> {
    let slot = match fontface_slot {
        Some(s) => s,
        None => return Ok(()),
    };
    let idx = *fontface_index;
    *fontface_index += 1;
    let face_name = match slot_face_name(doc, slot, idx) {
        Some(n) => n,
        None => return Ok(()),
    };
    let mut overrides = HashMap::new();
    overrides.insert("face".to_string(), face_name);
    replace_with_overlay(
        original, out, cursor,
        event_start, event_end,
        e, is_empty, &overrides,
    )
}

fn rewrite_strikeout(
    doc: &IrDocument,
    original: &[u8],
    out: &mut Vec<u8>,
    cursor: &mut usize,
    event_start: usize,
    event_end: usize,
    e: &BytesStart<'_>,
    is_empty: bool,
    char_pr_id: u32,
) -> Result<(), IrError> {
    let shape = match doc.doc_info.char_shapes.get(char_pr_id as usize) {
        Some(s) => s,
        None => return Ok(()),
    };
    let mut overrides = HashMap::new();
    overrides.insert(
        "shape".to_string(),
        if shape.strike() { "SOLID".to_string() } else { "NONE".to_string() },
    );
    if let Some(c) = shape.strike_color {
        overrides.insert("color".to_string(), color_to_hex(c));
    }
    replace_with_overlay(
        original, out, cursor,
        event_start, event_end,
        e, is_empty, &overrides,
    )
}

fn rewrite_underline(
    doc: &IrDocument,
    original: &[u8],
    out: &mut Vec<u8>,
    cursor: &mut usize,
    event_start: usize,
    event_end: usize,
    e: &BytesStart<'_>,
    is_empty: bool,
    char_pr_id: u32,
) -> Result<(), IrError> {
    let shape = match doc.doc_info.char_shapes.get(char_pr_id as usize) {
        Some(s) => s,
        None => return Ok(()),
    };
    let mut overrides = HashMap::new();
    overrides.insert(
        "shape".to_string(),
        if shape.underline_kind() == 0 { "NONE".to_string() } else { "SOLID".to_string() },
    );
    overrides.insert("color".to_string(), color_to_hex(shape.underline_color));
    replace_with_overlay(
        original, out, cursor,
        event_start, event_end,
        e, is_empty, &overrides,
    )
}

fn charpr_attr_overrides(shape: &CharShape) -> HashMap<String, String> {
    let mut o = HashMap::new();
    o.insert("height".to_string(), shape.base_size.to_string());
    o.insert("textColor".to_string(), color_to_hex(shape.color));
    o.insert("shadeColor".to_string(), color_to_hex(shape.shade_color));
    if let Some(id) = shape.border_fill_id {
        o.insert("borderFillIDRef".to_string(), id.to_string());
    }
    o
}

// ─── Byte-range splice ──────────────────────────────────────────────

fn replace_with_overlay(
    original: &[u8],
    out: &mut Vec<u8>,
    cursor: &mut usize,
    event_start: usize,
    event_end: usize,
    e: &BytesStart<'_>,
    is_empty: bool,
    overrides: &HashMap<String, String>,
) -> Result<(), IrError> {
    out.extend_from_slice(&original[*cursor..event_start]);
    emit_tag_with_overrides(e, is_empty, overrides, out)?;
    *cursor = event_end;
    Ok(())
}

fn emit_tag_with_overrides(
    e: &BytesStart<'_>,
    is_empty: bool,
    overrides: &HashMap<String, String>,
    out: &mut Vec<u8>,
) -> Result<(), IrError> {
    out.push(b'<');
    out.extend_from_slice(e.name().as_ref());
    for attr in e.attributes() {
        let attr = attr.map_err(|err| {
            IrError::Invalid(format!("hwpx header rewrite attr: {err}"))
        })?;
        let key = attr.key.as_ref();
        let key_str = std::str::from_utf8(key).unwrap_or("");
        out.push(b' ');
        out.extend_from_slice(key);
        out.extend_from_slice(b"=\"");
        if let Some(new_val) = overrides.get(key_str) {
            xml_escape_attr_value(new_val, out);
        } else {
            // Verbatim copy of the raw bytes so any entity references
            // (`&amp;`, `&lt;`) round-trip unchanged.
            out.extend_from_slice(&attr.value);
        }
        out.push(b'"');
    }
    if is_empty {
        out.extend_from_slice(b"/>");
    } else {
        out.push(b'>');
    }
    Ok(())
}

fn xml_escape_attr_value(s: &str, out: &mut Vec<u8>) {
    for c in s.chars() {
        match c {
            '<' => out.extend_from_slice(b"&lt;"),
            '>' => out.extend_from_slice(b"&gt;"),
            '&' => out.extend_from_slice(b"&amp;"),
            '"' => out.extend_from_slice(b"&quot;"),
            _ => {
                let mut b = [0u8; 4];
                let s = c.encode_utf8(&mut b);
                out.extend_from_slice(s.as_bytes());
            }
        }
    }
}

// ─── Lookups & encoding ─────────────────────────────────────────────

fn slot_face_name(doc: &IrDocument, slot: &'static str, idx: usize) -> Option<String> {
    let vec = match slot {
        "hangul" => &doc.doc_info.font_faces.hangul,
        "latin" => &doc.doc_info.font_faces.latin,
        "hanja" => &doc.doc_info.font_faces.hanja,
        "japanese" => &doc.doc_info.font_faces.japanese,
        "symbol" => &doc.doc_info.font_faces.symbol,
        "user" => &doc.doc_info.font_faces.user,
        _ => &doc.doc_info.font_faces.other,
    };
    vec.get(idx).map(|f| f.name.clone())
}

fn lang_to_slot(lang: Option<&str>) -> Option<&'static str> {
    Some(match lang? {
        "HANGUL" => "hangul",
        "LATIN" => "latin",
        "HANJA" => "hanja",
        "JAPANESE" => "japanese",
        "SYMBOL" => "symbol",
        "USER" => "user",
        _ => "other",
    })
}

fn align_to_hwpx(bits: u8) -> &'static str {
    match bits {
        0 => "JUSTIFY",
        1 => "LEFT",
        2 => "RIGHT",
        3 => "CENTER",
        4 => "DISTRIBUTE",
        5 => "DISTRIBUTE_SPACE",
        _ => "LEFT",
    }
}

/// Pack u32 colour (HWP5 layout: byte0=R, byte1=G, byte2=B) → `#RRGGBB`.
fn color_to_hex(c: u32) -> String {
    let r = (c & 0xFF) as u8;
    let g = ((c >> 8) & 0xFF) as u8;
    let b = ((c >> 16) & 0xFF) as u8;
    format!("#{:02X}{:02X}{:02X}", r, g, b)
}

// ─── XML helpers ────────────────────────────────────────────────────

fn local_name<'a>(e: &'a BytesStart<'_>) -> &'a str {
    local_name_from_bytes(e.name().into_inner())
}

fn local_name_from_bytes(bytes: &[u8]) -> &str {
    let start = bytes
        .iter()
        .position(|&b| b == b':')
        .map(|i| i + 1)
        .unwrap_or(0);
    std::str::from_utf8(&bytes[start..]).unwrap_or("")
}

fn u32_attr(e: &BytesStart<'_>, name: &str) -> Option<u32> {
    string_attr(e, name)?.trim().parse::<u32>().ok()
}

fn string_attr(e: &BytesStart<'_>, name: &str) -> Option<String> {
    for attr in e.attributes().flatten() {
        if attr.key.as_ref() == name.as_bytes() {
            return std::str::from_utf8(&attr.value).ok().map(|s| s.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use hwp_transpiler_core::ir::{
        BorderFill, CharShape, Fill, FontFace, FontFaces, ParaShape,
    };

    fn doc_with_para_shapes(shapes: Vec<ParaShape>) -> IrDocument {
        let mut doc = IrDocument::default();
        doc.doc_info.para_shapes = shapes;
        doc
    }

    fn ps_with_align(bits: u8) -> ParaShape {
        let mut p = ParaShape::default();
        p.attribute = bits as u32;
        p
    }

    #[test]
    fn align_value_overlaid_from_ir() {
        let xml = br##"<hh:head xmlns:hh="h">
            <hh:refList>
              <hh:paraProperties itemCnt="1">
                <hh:paraPr id="0">
                  <hh:align horizontal="LEFT" vertical="BASELINE"/>
                </hh:paraPr>
              </hh:paraProperties>
            </hh:refList>
          </hh:head>"##;
        let doc = doc_with_para_shapes(vec![ps_with_align(2)]); // 2 = RIGHT
        let out = rewrite_header_xml(xml, &doc).expect("rewrite");
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains(r#"horizontal="RIGHT""#), "expected RIGHT, got: {s}");
        assert!(s.contains(r#"vertical="BASELINE""#),
            "vertical should be preserved verbatim: {s}");
    }

    #[test]
    fn align_outside_parapr_is_not_touched() {
        // An `<hh:align>` element outside `<hh:paraPr>` should NOT be
        // rewritten — only paraPr's child gets overlaid.
        let xml = br##"<hh:head xmlns:hh="h">
            <hh:bogusOuter>
              <hh:align horizontal="LEFT"/>
            </hh:bogusOuter>
          </hh:head>"##;
        let doc = doc_with_para_shapes(vec![ps_with_align(2)]);
        let out = rewrite_header_xml(xml, &doc).expect("rewrite");
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains(r#"horizontal="LEFT""#), "outer align kept: {s}");
    }

    #[test]
    fn fontface_face_overlaid_from_ir() {
        let xml = br##"<hh:head xmlns:hh="h">
            <hh:fontfaces>
              <hh:fontface lang="HANGUL">
                <hh:font id="0" face="OldName" type="TTF" isEmbedded="0"/>
              </hh:fontface>
            </hh:fontfaces>
          </hh:head>"##;
        let mut doc = IrDocument::default();
        doc.doc_info.font_faces = FontFaces {
            hangul: vec![FontFace {
                properties: 0,
                name: "NewName".into(),
                substitute: None,
                type_info: None,
                default_name: None,
            }],
            ..FontFaces::default()
        };
        let out = rewrite_header_xml(xml, &doc).expect("rewrite");
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains(r#"face="NewName""#), "expected NewName: {s}");
        // Other attrs preserved.
        assert!(s.contains(r#"type="TTF""#), "type preserved: {s}");
        assert!(s.contains(r#"isEmbedded="0""#), "isEmbedded preserved: {s}");
    }

    #[test]
    fn charpr_height_and_color_overlaid() {
        let xml = br##"<hh:head xmlns:hh="h">
            <hh:charProperties>
              <hh:charPr id="0" height="1000" textColor="#000000" shadeColor="#FFFFFF" useFontSpace="0">
                <hh:fontRef hangul="0" latin="0" hanja="0" japanese="0" other="0" symbol="0" user="0"/>
              </hh:charPr>
            </hh:charProperties>
          </hh:head>"##;
        let mut doc = IrDocument::default();
        let mut cs = CharShape::default();
        cs.base_size = 1500;
        // 0xAABBCC packed: R=0xCC, G=0xBB, B=0xAA → emit "#CCBBAA"
        cs.color = 0x00AA_BBCC;
        cs.shade_color = 0x00FFFFFF;
        doc.doc_info.char_shapes = vec![cs];
        let out = rewrite_header_xml(xml, &doc).expect("rewrite");
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains(r#"height="1500""#), "height overlaid: {s}");
        assert!(s.contains(r##"textColor="#CCBBAA""##), "textColor overlaid: {s}");
        // useFontSpace="0" should survive verbatim — we never touch it.
        assert!(s.contains(r#"useFontSpace="0""#),
            "unrelated attr preserved: {s}");
        // fontRef child preserved verbatim.
        assert!(s.contains("<hh:fontRef"), "fontRef preserved: {s}");
    }

    #[test]
    fn strikeout_and_underline_shape_attr_overlaid() {
        let xml = br##"<hh:head xmlns:hh="h">
            <hh:charProperties>
              <hh:charPr id="0" height="1000" textColor="#000000">
                <hh:fontRef hangul="0" latin="0" hanja="0" japanese="0" other="0" symbol="0" user="0"/>
                <hh:strikeout shape="NONE" color="#000000"/>
                <hh:underline shape="NONE" color="#000000"/>
              </hh:charPr>
            </hh:charProperties>
          </hh:head>"##;
        let mut doc = IrDocument::default();
        let mut cs = CharShape::default();
        cs.attr = (1 << 21) | 0x0000_0004; // strike + underline kind=1
        cs.strike_color = Some(0x0000_00FF); // R=FF G=00 B=00 → "#FF0000"
        cs.underline_color = 0x00FF_0000; // R=00 G=00 B=FF → "#0000FF"
        doc.doc_info.char_shapes = vec![cs];
        let out = rewrite_header_xml(xml, &doc).expect("rewrite");
        let s = std::str::from_utf8(&out).unwrap();
        assert!(
            s.contains(r##"<hh:strikeout shape="SOLID" color="#FF0000"/>"##),
            "strike overlay: {s}"
        );
        assert!(
            s.contains(r##"<hh:underline shape="SOLID" color="#0000FF"/>"##),
            "underline overlay: {s}"
        );
    }

    #[test]
    fn winbrush_face_color_overlaid_for_solid_fill() {
        let xml = br##"<hh:head xmlns:hh="h" xmlns:hc="c">
            <hh:borderFills>
              <hh:borderFill id="1">
                <hc:fillBrush>
                  <hc:winBrush faceColor="#000000" hatchColor="#999999" alpha="0"/>
                </hc:fillBrush>
              </hh:borderFill>
            </hh:borderFills>
          </hh:head>"##;
        let mut doc = IrDocument::default();
        let mut bf = BorderFill::default();
        bf.fill = Fill {
            kind: Fill::KIND_COLOR,
            body: vec![0xCC, 0xDD, 0xEE, 0],
        };
        // Pad slot 0 + slot 1.
        doc.doc_info.border_fills = vec![BorderFill::default(), bf];
        let out = rewrite_header_xml(xml, &doc).expect("rewrite");
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains(r##"faceColor="#CCDDEE""##), "faceColor overlay: {s}");
        // hatchColor + alpha preserved verbatim.
        assert!(s.contains(r##"hatchColor="#999999""##), "hatch preserved: {s}");
        assert!(s.contains(r#"alpha="0""#), "alpha preserved: {s}");
    }

    #[test]
    fn missing_ir_shape_leaves_event_verbatim() {
        // paraPr id=5 but IR only has 1 paraShape → no overlay.
        let xml = br##"<hh:head xmlns:hh="h">
            <hh:paraProperties>
              <hh:paraPr id="5">
                <hh:align horizontal="LEFT"/>
              </hh:paraPr>
            </hh:paraProperties>
          </hh:head>"##;
        let doc = doc_with_para_shapes(vec![ps_with_align(2)]);
        let out = rewrite_header_xml(xml, &doc).expect("rewrite");
        let s = std::str::from_utf8(&out).unwrap();
        // The byte range outside paraPr id=5's IR slot is left alone.
        assert!(s.contains(r#"horizontal="LEFT""#),
            "out-of-range IR should not touch: {s}");
    }

    #[test]
    fn unknown_attr_value_with_entity_round_trips() {
        // If an unrelated attribute value contains entity references,
        // the verbatim copy path must preserve them so the rewriter
        // doesn't silently double-escape.
        let xml = br##"<hh:head xmlns:hh="h">
            <hh:fontfaces>
              <hh:fontface lang="HANGUL">
                <hh:font id="0" face="A &amp; B" type="TTF"/>
              </hh:fontface>
            </hh:fontfaces>
          </hh:head>"##;
        let mut doc = IrDocument::default();
        doc.doc_info.font_faces.hangul = vec![FontFace {
            properties: 0,
            name: "A & B".into(), // same logical name
            substitute: None,
            type_info: None,
            default_name: None,
        }];
        let out = rewrite_header_xml(xml, &doc).expect("rewrite");
        let s = std::str::from_utf8(&out).unwrap();
        // Our overlay re-emits `face="A & B"` — XML-escaped → `&amp;`.
        assert!(s.contains(r#"face="A &amp; B""#),
            "ampersand re-escaped on overlay: {s}");
        // type attr's value was never overridden, so its bytes flow
        // through; this incidentally exercises the verbatim path.
        assert!(s.contains(r#"type="TTF""#));
    }
}
