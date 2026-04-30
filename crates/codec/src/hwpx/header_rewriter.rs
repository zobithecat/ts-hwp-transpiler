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
//! Supported overlays:
//!   * `<hh:align horizontal=…>` inside `<hh:paraPr>` — from
//!     `ParaShape::align()`.
//!   * `<hh:charPr height=… textColor=… shadeColor=… borderFillIDRef=…>`
//!     — from `CharShape` parent attrs (only attrs already present on
//!     the element are touched).
//!   * `<hh:strikeout shape=… color=…>` and `<hh:underline shape=… color=…>`
//!     inside `<hh:charPr>` — from `CharShape::strike()`/
//!     `underline_kind()` and the matching colour fields. Inserted
//!     before `</hh:charPr>` when the IR wants the flag on but the
//!     original document never carried the element.
//!   * `<hh:fontRef>`, `<hh:ratio>`, `<hh:relSz>`, `<hh:spacing>`,
//!     `<hh:offset>` (multi-script 7-attribute children of
//!     `<hh:charPr>`) — from `CharShape::font_ids` / `ratios` /
//!     `rel_sizes` / `char_spacings` / `char_offsets`.
//!   * `<hh:bold/>` / `<hh:italic/>` (presence-only children of
//!     `<hh:charPr>`) — IR-on inserts before `</hh:charPr>` when the
//!     original lacked the element; IR-off skips an existing event's
//!     bytes from output.
//!   * `<hh:font face=…>` inside `<hh:fontface>` — from
//!     `FontFace::name`.
//!   * `<hc:winBrush faceColor=…>` inside `<hh:borderFill>` — from
//!     the IR's solid colour fill (gradation / image fills are left
//!     alone).
//!
//! Add / remove of whole `<hh:paraPr>` / `<hh:charPr>` shapes is
//! supported via two complementary paths:
//!
//!   * **Removal**: an original `<hh:paraPr id=N>` / `<hh:charPr id=N>`
//!     whose `id` is past the IR vec's length is treated as
//!     IR-deleted; we swallow the entire span (Start through End)
//!     by entering a depth-tracked skip state.
//!   * **Addition**: as we walk the original we collect the ids that
//!     appeared. On `</hh:paraProperties>` / `</hh:charProperties>`
//!     End we emit a fresh full block for every IR-side id that
//!     wasn't seen, spliced before the container's End tag. New
//!     paraPrs ride a minimal `<hh:align>` child; new charPrs emit
//!     every child the parser expects (fontRef / ratio / relSz /
//!     spacing / offset / bold-italic-strikeout-underline) so a
//!     re-parse populates the same IR state.
//!
//! Not yet handled — fontface add / remove (per-script slot
//! partitioning needs separate accounting), gradation / image fill
//! mutation (the IR doesn't reflect them as typed fields). The
//! unmutated path round-trips these correctly because the rewriter
//! never touches their bytes.

use std::collections::{HashMap, HashSet};

use hwp_transpiler_core::ir::{
    Border, BorderFill, CharShape, Fill, IrDocument, IrError, ParaShape, Style,
};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

/// Active span-skip — set when we encounter `<hh:paraPr id=N>` /
/// `<hh:charPr id=N>` whose id is past the IR's vec length (the user
/// removed it from IR). We swallow the entire span up to and
/// including the matching End tag.
struct SkipState {
    /// Local element name being skipped (`"paraPr"` or `"charPr"`).
    elem_name: String,
    /// Same-name nesting depth — defensive, paraPr / charPr don't
    /// actually nest in HWPX but a malformed input could.
    depth: usize,
}

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

    // Track which presence-driven children appeared inside the current
    // `<hh:charPr>`. On `</hh:charPr>` end we insert any missing ones
    // that the IR wants enabled, and on entry we skip bold / italic
    // events the IR wants disabled.
    let mut seen_bold = false;
    let mut seen_italic = false;
    let mut seen_strikeout = false;
    let mut seen_underline = false;

    // Add / remove of whole shapes. `container` is set when inside
    // `<hh:paraProperties>` or `<hh:charProperties>` so we can collect
    // the ids that appeared and emit any missing IR-side ids before
    // the container's End tag. `skip` swallows a paraPr / charPr span
    // whose id is past the IR's vec length (the IR side dropped it).
    // `container` is set when we're inside `<hh:paraProperties>` or
    // `<hh:charProperties>`; the End-tag handler distinguishes which
    // by element name. We never nest these containers in HWPX so a
    // single Option suffices.
    let mut container: Option<HashSet<u32>> = None;
    // Styles live in a sibling container (`<hh:styles>`); track its
    // own seen-id set so we can splice missing IR-side styles before
    // its End. Separate from `container` because the element name
    // differs and we may technically encounter both in any order.
    let mut styles_container: Option<HashSet<u32>> = None;
    // Border fills sit in their own `<hh:borderFills>` container.
    // When the original ships fewer entries than the IR carries
    // (HWP5-sourced docs typically have 30+ borderFills, the bundled
    // skeleton has only 2), we need to splice the missing ones before
    // the closing tag — otherwise table cells reference IDs that
    // don't resolve and viewers render them with no borders.
    let mut border_fills_container: Option<HashSet<u32>> = None;
    let mut skip: Option<SkipState> = None;

    let mut buf = Vec::new();

    loop {
        let event_start = reader.buffer_position() as usize;
        let evt = reader
            .read_event_into(&mut buf)
            .map_err(|e| IrError::Invalid(format!("hwpx header rewrite: {e}")))?;
        let event_end = reader.buffer_position() as usize;

        // Skip mode: swallow every event until we hit the matching
        // End tag at depth 0. The whole span is dropped from output
        // because we set `cursor = event_end` on entry and again on
        // exit, so nothing in between flushes.
        if let Some(s) = skip.as_mut() {
            match evt {
                Event::Start(ref e) if local_name(e) == s.elem_name => {
                    s.depth += 1;
                }
                Event::End(ref e)
                    if local_name_from_bytes(e.name().as_ref()) == s.elem_name =>
                {
                    s.depth -= 1;
                    if s.depth == 0 {
                        cursor = event_end;
                        skip = None;
                    }
                }
                Event::Eof => {
                    return Err(IrError::Invalid(
                        "hwpx header rewrite: EOF while skipping span".into(),
                    ));
                }
                _ => {}
            }
            buf.clear();
            continue;
        }

        match evt {
            Event::Start(ref e) => {
                let name = local_name(e);
                match name {
                    "paraProperties" => {
                        container = Some(HashSet::new());
                    }
                    "charProperties" => {
                        container = Some(HashSet::new());
                    }
                    "styles" => {
                        styles_container = Some(HashSet::new());
                    }
                    "borderFills" => {
                        border_fills_container = Some(HashSet::new());
                    }
                    "style" if styles_container.is_some() => {
                        // Rare Start variant `<hh:style ...>...</hh:style>`.
                        // Track id; skip the span if IR removed it.
                        let id = u32_attr(e, "id");
                        if let (Some(id), Some(c)) = (id, styles_container.as_mut()) {
                            c.insert(id);
                        }
                        if let Some(id) = id {
                            if (id as usize) >= doc.doc_info.styles.len() {
                                out.extend_from_slice(&original[cursor..event_start]);
                                cursor = event_end;
                                skip = Some(SkipState {
                                    elem_name: "style".into(),
                                    depth: 1,
                                });
                                buf.clear();
                                continue;
                            }
                        }
                    }
                    "paraPr" => {
                        let id = u32_attr(e, "id");
                        if let (Some(id), Some(c)) = (id, container.as_mut()) {
                            c.insert(id);
                        }
                        // IR-side removed this id → swallow the span.
                        if let Some(id) = id {
                            if (id as usize) >= doc.doc_info.para_shapes.len() {
                                out.extend_from_slice(&original[cursor..event_start]);
                                cursor = event_end;
                                skip = Some(SkipState {
                                    elem_name: "paraPr".into(),
                                    depth: 1,
                                });
                                buf.clear();
                                continue;
                            }
                        }
                        para_pr_id = id;
                    }
                    "charPr" => {
                        let id = u32_attr(e, "id");
                        if let (Some(id), Some(c)) = (id, container.as_mut()) {
                            c.insert(id);
                        }
                        if let Some(id) = id {
                            if (id as usize) >= doc.doc_info.char_shapes.len() {
                                out.extend_from_slice(&original[cursor..event_start]);
                                cursor = event_end;
                                skip = Some(SkipState {
                                    elem_name: "charPr".into(),
                                    depth: 1,
                                });
                                buf.clear();
                                continue;
                            }
                        }
                        char_pr_id = id;
                        seen_bold = false;
                        seen_italic = false;
                        seen_strikeout = false;
                        seen_underline = false;
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
                        let id = u32_attr(e, "id");
                        border_fill_id = id;
                        // Track the id in the borderFills container so
                        // the End-of-`<hh:borderFills>` handler can
                        // append only the IR entries the original
                        // didn't already carry.
                        if let (Some(id), Some(c)) =
                            (id, border_fills_container.as_mut())
                        {
                            c.insert(id);
                        }
                    }
                    "strikeout" if char_pr_id.is_some() => {
                        seen_strikeout = true;
                        rewrite_strikeout(
                            doc, original, &mut out, &mut cursor,
                            event_start, event_end,
                            e, false, char_pr_id.unwrap(),
                        )?;
                    }
                    "underline" if char_pr_id.is_some() => {
                        seen_underline = true;
                        rewrite_underline(
                            doc, original, &mut out, &mut cursor,
                            event_start, event_end,
                            e, false, char_pr_id.unwrap(),
                        )?;
                    }
                    "bold" if char_pr_id.is_some() => {
                        // Rare: `<hh:bold>` Start variant. Mark as
                        // seen so the End handler doesn't insert a
                        // duplicate. We don't skip the bytes because
                        // we can't safely strip a Start/End span
                        // here; the unmutated path keeps it intact.
                        seen_bold = true;
                    }
                    "italic" if char_pr_id.is_some() => {
                        seen_italic = true;
                    }
                    "fontRef" | "ratio" | "relSz" | "spacing" | "offset"
                        if char_pr_id.is_some() =>
                    {
                        rewrite_charpr_array_child(
                            doc, original, &mut out, &mut cursor,
                            event_start, event_end,
                            e, false, char_pr_id.unwrap(), name,
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
                        if let (Some(id), Some(c)) = (id, container.as_mut()) {
                            c.insert(id);
                        }
                        if let Some(id) = id {
                            if (id as usize) >= doc.doc_info.char_shapes.len() {
                                skip_event_bytes(
                                    original, &mut out, &mut cursor,
                                    event_start, event_end,
                                );
                            } else if let Some(shape) =
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
                    "paraPr" => {
                        // Empty paraPr (no children). Same id-check
                        // pattern as the Start variant — skip when
                        // the IR removed it.
                        let id = u32_attr(e, "id");
                        if let (Some(id), Some(c)) = (id, container.as_mut()) {
                            c.insert(id);
                        }
                        if let Some(id) = id {
                            if (id as usize) >= doc.doc_info.para_shapes.len() {
                                skip_event_bytes(
                                    original, &mut out, &mut cursor,
                                    event_start, event_end,
                                );
                            }
                            // Otherwise leave the empty event verbatim
                            // — there's no align child to overlay.
                        }
                    }
                    "style" if styles_container.is_some() => {
                        // Self-closing `<hh:style id=N/>` is the
                        // typical form. Track the id and drop it if
                        // IR has truncated past it. Other attributes
                        // (name / paraPrIDRef / charPrIDRef …) flow
                        // through verbatim — IR-side mutation of an
                        // existing style isn't supported in this pass
                        // because the parser doesn't surface the
                        // styles container to the IR.
                        let id = u32_attr(e, "id");
                        if let (Some(id), Some(c)) = (id, styles_container.as_mut()) {
                            c.insert(id);
                        }
                        if let Some(id) = id {
                            if (id as usize) >= doc.doc_info.styles.len() {
                                skip_event_bytes(
                                    original, &mut out, &mut cursor,
                                    event_start, event_end,
                                );
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
                        seen_strikeout = true;
                        rewrite_strikeout(
                            doc, original, &mut out, &mut cursor,
                            event_start, event_end,
                            e, true, char_pr_id.unwrap(),
                        )?;
                    }
                    "underline" if char_pr_id.is_some() => {
                        seen_underline = true;
                        rewrite_underline(
                            doc, original, &mut out, &mut cursor,
                            event_start, event_end,
                            e, true, char_pr_id.unwrap(),
                        )?;
                    }
                    "bold" if char_pr_id.is_some() => {
                        seen_bold = true;
                        let id = char_pr_id.unwrap();
                        if let Some(shape) =
                            doc.doc_info.char_shapes.get(id as usize)
                        {
                            if !shape.bold() {
                                // IR says bold off — skip the event
                                // bytes so the original `<hh:bold/>`
                                // doesn't reappear in output.
                                skip_event_bytes(
                                    original, &mut out, &mut cursor,
                                    event_start, event_end,
                                );
                            }
                        }
                    }
                    "italic" if char_pr_id.is_some() => {
                        seen_italic = true;
                        let id = char_pr_id.unwrap();
                        if let Some(shape) =
                            doc.doc_info.char_shapes.get(id as usize)
                        {
                            if !shape.italic() {
                                skip_event_bytes(
                                    original, &mut out, &mut cursor,
                                    event_start, event_end,
                                );
                            }
                        }
                    }
                    "fontRef" | "ratio" | "relSz" | "spacing" | "offset"
                        if char_pr_id.is_some() =>
                    {
                        rewrite_charpr_array_child(
                            doc, original, &mut out, &mut cursor,
                            event_start, event_end,
                            e, true, char_pr_id.unwrap(), name,
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
                    "paraProperties" => {
                        // Splice any IR-side paraShapes whose id wasn't
                        // emitted yet (i.e. user pushed new shapes onto
                        // the IR vec) before the container's End tag.
                        if let Some(c) = container.as_ref() {
                            let mut insertion = Vec::new();
                            for (idx, shape) in
                                doc.doc_info.para_shapes.iter().enumerate()
                            {
                                let id = idx as u32;
                                if !c.contains(&id) {
                                    emit_new_para_pr(id, shape, &mut insertion);
                                }
                            }
                            if !insertion.is_empty() {
                                insert_before_event(
                                    original, &mut out, &mut cursor,
                                    event_start, event_end,
                                    &insertion,
                                );
                            }
                        }
                        container = None;
                    }
                    "charProperties" => {
                        if let Some(c) = container.as_ref() {
                            let mut insertion = Vec::new();
                            for (idx, shape) in
                                doc.doc_info.char_shapes.iter().enumerate()
                            {
                                let id = idx as u32;
                                if !c.contains(&id) {
                                    emit_new_char_pr(id, shape, &mut insertion);
                                }
                            }
                            if !insertion.is_empty() {
                                insert_before_event(
                                    original, &mut out, &mut cursor,
                                    event_start, event_end,
                                    &insertion,
                                );
                            }
                        }
                        container = None;
                    }
                    "borderFills" => {
                        if let Some(c) = border_fills_container.as_ref() {
                            let mut insertion = Vec::new();
                            for (idx, fill) in
                                doc.doc_info.border_fills.iter().enumerate()
                            {
                                let id = idx as u32;
                                if !c.contains(&id) {
                                    emit_new_border_fill(id, fill, &mut insertion);
                                }
                            }
                            if !insertion.is_empty() {
                                insert_before_event(
                                    original, &mut out, &mut cursor,
                                    event_start, event_end,
                                    &insertion,
                                );
                            }
                        }
                        border_fills_container = None;
                    }
                    "styles" => {
                        if let Some(c) = styles_container.as_ref() {
                            let mut insertion = Vec::new();
                            for (idx, style) in
                                doc.doc_info.styles.iter().enumerate()
                            {
                                let id = idx as u32;
                                if !c.contains(&id) {
                                    emit_new_style(id, style, &mut insertion);
                                }
                            }
                            if !insertion.is_empty() {
                                insert_before_event(
                                    original, &mut out, &mut cursor,
                                    event_start, event_end,
                                    &insertion,
                                );
                            }
                        }
                        styles_container = None;
                    }
                    "paraPr" => para_pr_id = None,
                    "charPr" => {
                        // Insert any presence-driven children the IR
                        // wants on but the original lacked. Keeps
                        // `</hh:charPr>` byte range intact — we splice
                        // before it.
                        if let Some(id) = char_pr_id {
                            if let Some(shape) =
                                doc.doc_info.char_shapes.get(id as usize)
                            {
                                let insertion = build_charpr_missing_inserts(
                                    shape,
                                    seen_bold,
                                    seen_italic,
                                    seen_strikeout,
                                    seen_underline,
                                );
                                if !insertion.is_empty() {
                                    insert_before_event(
                                        original, &mut out, &mut cursor,
                                        event_start, event_end,
                                        &insertion,
                                    );
                                }
                            }
                        }
                        char_pr_id = None;
                        seen_bold = false;
                        seen_italic = false;
                        seen_strikeout = false;
                        seen_underline = false;
                    }
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

/// Slot order for `<hh:fontRef>` / `<hh:ratio>` / `<hh:relSz>` /
/// `<hh:spacing>` / `<hh:offset>`. Mirrors the parser's
/// `SCRIPT_SLOTS` so the IR's 7-element arrays line up with the
/// HWPX attribute names.
const SCRIPT_SLOTS: [&str; 7] = [
    "hangul", "latin", "hanja", "japanese", "other", "symbol", "user",
];

fn rewrite_charpr_array_child(
    doc: &IrDocument,
    original: &[u8],
    out: &mut Vec<u8>,
    cursor: &mut usize,
    event_start: usize,
    event_end: usize,
    e: &BytesStart<'_>,
    is_empty: bool,
    char_pr_id: u32,
    elem_name: &str,
) -> Result<(), IrError> {
    let shape = match doc.doc_info.char_shapes.get(char_pr_id as usize) {
        Some(s) => s,
        None => return Ok(()),
    };
    let mut overrides = HashMap::new();
    match elem_name {
        "fontRef" => {
            for (i, slot) in SCRIPT_SLOTS.iter().enumerate() {
                overrides.insert(slot.to_string(), shape.font_ids[i].to_string());
            }
        }
        "ratio" => {
            for (i, slot) in SCRIPT_SLOTS.iter().enumerate() {
                overrides.insert(slot.to_string(), shape.ratios[i].to_string());
            }
        }
        "relSz" => {
            for (i, slot) in SCRIPT_SLOTS.iter().enumerate() {
                overrides.insert(slot.to_string(), shape.rel_sizes[i].to_string());
            }
        }
        "spacing" => {
            for (i, slot) in SCRIPT_SLOTS.iter().enumerate() {
                overrides.insert(
                    slot.to_string(),
                    shape.char_spacings[i].to_string(),
                );
            }
        }
        "offset" => {
            for (i, slot) in SCRIPT_SLOTS.iter().enumerate() {
                overrides.insert(
                    slot.to_string(),
                    shape.char_offsets[i].to_string(),
                );
            }
        }
        _ => return Ok(()),
    }
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
    // HWPX uses the literal `"none"` sentinel for "no shading"; our
    // IR collapses that to `shade_color = 0`, which would round-trip
    // as `#000000` (literal black) and viewers paint every glyph
    // background black. Treat 0 as the sentinel and emit `"none"`.
    o.insert(
        "shadeColor".to_string(),
        if shape.shade_color == 0 {
            "none".to_string()
        } else {
            color_to_hex(shape.shade_color)
        },
    );
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

/// Drop an event's bytes from the output entirely. Whitespace before
/// the event (in a separate `Event::Text`) still flows through, which
/// can leave a tiny extra newline / indent where the element used to
/// be — harmless to readers, slightly noisy on byte diff.
fn skip_event_bytes(
    original: &[u8],
    out: &mut Vec<u8>,
    cursor: &mut usize,
    event_start: usize,
    event_end: usize,
) {
    out.extend_from_slice(&original[*cursor..event_start]);
    *cursor = event_end;
}

/// Splice arbitrary bytes in just before an event (typically an End
/// tag) without disturbing the event's own bytes.
fn insert_before_event(
    original: &[u8],
    out: &mut Vec<u8>,
    cursor: &mut usize,
    event_start: usize,
    event_end: usize,
    insertion: &[u8],
) {
    out.extend_from_slice(&original[*cursor..event_start]);
    out.extend_from_slice(insertion);
    out.extend_from_slice(&original[event_start..event_end]);
    *cursor = event_end;
}

/// Emit a from-scratch `<hh:style>` element for an IR-side style.
/// HWPX style schema: `id`, `type` (PARA / CHAR), `name`, `engName`,
/// `paraPrIDRef`, `charPrIDRef`, `lockForm`. We always emit them as
/// self-closing — the schema allows children but our IR doesn't carry
/// any.
fn emit_new_style(id: u32, style: &Style, out: &mut Vec<u8>) {
    let style_type = if (style.properties & 0x07) == 1 { "CHAR" } else { "PARA" };
    out.extend_from_slice(b"<hh:style id=\"");
    out.extend_from_slice(id.to_string().as_bytes());
    out.extend_from_slice(b"\" type=\"");
    out.extend_from_slice(style_type.as_bytes());
    out.extend_from_slice(b"\" name=\"");
    let mut escaped = Vec::new();
    xml_escape_attr_value(&style.name, &mut escaped);
    out.extend_from_slice(&escaped);
    out.extend_from_slice(b"\" engName=\"");
    let mut escaped = Vec::new();
    xml_escape_attr_value(&style.english_name, &mut escaped);
    out.extend_from_slice(&escaped);
    out.extend_from_slice(b"\" paraPrIDRef=\"");
    out.extend_from_slice(style.para_shape_id.to_string().as_bytes());
    out.extend_from_slice(b"\" charPrIDRef=\"");
    out.extend_from_slice(style.char_shape_id.to_string().as_bytes());
    out.extend_from_slice(b"\" lockForm=\"0\" nextStyleIDRef=\"");
    out.extend_from_slice(style.next_style_id.to_string().as_bytes());
    out.extend_from_slice(b"\" langID=\"");
    out.extend_from_slice(style.lang_id.to_string().as_bytes());
    out.extend_from_slice(b"\"/>");
}

/// Emit a from-scratch `<hh:paraPr>` block for an IR-side paraShape
/// the original document didn't carry. Hancom-authored paraPrs ship
/// a full child set (align, heading, breakSetting, margin, lineSpacing,
/// border, autoSpacing); without those, mac HWP 2014 / rhwp default
/// to broken layout (zero line height, missing word breaks, no
/// margins) and tables ride along with the same broken metrics. The
/// IR's ParaShape only carries `align` typed for now — the other
/// values use Hancom's "single-spaced 10pt body text" defaults.
fn emit_new_para_pr(id: u32, shape: &ParaShape, out: &mut Vec<u8>) {
    out.extend_from_slice(b"<hh:paraPr id=\"");
    out.extend_from_slice(id.to_string().as_bytes());
    out.extend_from_slice(
        concat!(
            r#"" tabPrIDRef="0" condense="0" fontLineHeight="0" snapToGrid="1" "#,
            r#"suppressLineNumbers="0" checked="0">"#,
            r#"<hh:align horizontal=""#,
        ).as_bytes(),
    );
    out.extend_from_slice(align_to_hwpx(shape.align()).as_bytes());
    out.extend_from_slice(
        concat!(
            r#"" vertical="BASELINE"/>"#,
            r#"<hh:heading type="NONE" idRef="0" level="0"/>"#,
            r#"<hh:breakSetting breakLatinWord="KEEP_WORD" breakNonLatinWord="KEEP_WORD" widowOrphan="0" keepWithNext="0" keepLines="0" pageBreakBefore="0" lineWrap="BREAK"/>"#,
            r#"<hh:margin intent="0" left="0" right="0" prev="0" next="0"/>"#,
            r#"<hh:lineSpacing type="PERCENT" value="160" unit="HWPUNIT"/>"#,
            r#"<hh:border borderFillIDRef="0" offsetLeft="0" offsetRight="0" offsetTop="0" offsetBottom="0" connect="0" ignoreMargin="0"/>"#,
            r#"<hh:autoSpacing eAsianEng="0" eAsianNum="0"/>"#,
            r#"</hh:paraPr>"#,
        ).as_bytes(),
    );
}

/// Emit a from-scratch `<hh:charPr>` block. Includes every child the
/// reader expects (fontRef / ratio / relSz / spacing / offset /
/// bold / italic / strikeout / underline) so a re-parse populates the
/// same IR state. `borderFillIDRef` is included only when the IR
/// carries an Option<u16>.
fn emit_new_char_pr(id: u32, shape: &CharShape, out: &mut Vec<u8>) {
    out.extend_from_slice(b"<hh:charPr id=\"");
    out.extend_from_slice(id.to_string().as_bytes());
    out.extend_from_slice(b"\" height=\"");
    out.extend_from_slice(shape.base_size.to_string().as_bytes());
    out.extend_from_slice(b"\" textColor=\"");
    out.extend_from_slice(color_to_hex(shape.color).as_bytes());
    out.extend_from_slice(b"\" shadeColor=\"");
    if shape.shade_color == 0 {
        out.extend_from_slice(b"none");
    } else {
        out.extend_from_slice(color_to_hex(shape.shade_color).as_bytes());
    }
    out.extend_from_slice(b"\"");
    if let Some(bf) = shape.border_fill_id {
        out.extend_from_slice(b" borderFillIDRef=\"");
        out.extend_from_slice(bf.to_string().as_bytes());
        out.push(b'"');
    }
    out.push(b'>');

    emit_script_array(b"fontRef", &shape.font_ids, out);
    emit_script_array(b"ratio", &shape.ratios, out);
    emit_script_array(b"relSz", &shape.rel_sizes, out);
    emit_script_array(b"spacing", &shape.char_spacings, out);
    emit_script_array(b"offset", &shape.char_offsets, out);

    if shape.bold() {
        out.extend_from_slice(b"<hh:bold/>");
    }
    if shape.italic() {
        out.extend_from_slice(b"<hh:italic/>");
    }
    let strike_shape = if shape.strike() { "SOLID" } else { "NONE" };
    let strike_color = color_to_hex(shape.strike_color.unwrap_or(0));
    out.extend_from_slice(b"<hh:strikeout shape=\"");
    out.extend_from_slice(strike_shape.as_bytes());
    out.extend_from_slice(b"\" color=\"");
    out.extend_from_slice(strike_color.as_bytes());
    out.extend_from_slice(b"\"/>");

    let underline_shape = if shape.underline_kind() != 0 { "SOLID" } else { "NONE" };
    out.extend_from_slice(b"<hh:underline shape=\"");
    out.extend_from_slice(underline_shape.as_bytes());
    out.extend_from_slice(b"\" color=\"");
    out.extend_from_slice(color_to_hex(shape.underline_color).as_bytes());
    out.extend_from_slice(b"\"/>");

    out.extend_from_slice(b"</hh:charPr>");
}

/// Emit a from-scratch `<hh:borderFill>` for an IR-side BorderFill the
/// original document didn't carry. HWP5-sourced docs commonly ship
/// 30+ borderFills (one per cell-style combination) but the bundled
/// HWPX skeleton only has 2 — table cells whose `borderFillIDRef`
/// points past the skeleton end render with no borders. Emit the full
/// shape: `slash` / `backSlash` / 4 borders + `diagonal` + `fillBrush`
/// (when the IR's Fill is a solid color), matching what Hancom-
/// authored docs ship.
fn emit_new_border_fill(id: u32, fill: &BorderFill, out: &mut Vec<u8>) {
    out.extend_from_slice(b"<hh:borderFill id=\"");
    out.extend_from_slice(id.to_string().as_bytes());
    out.extend_from_slice(
        b"\" threeD=\"0\" shadow=\"0\" centerLine=\"NONE\" breakCellSeparateLine=\"0\">",
    );
    out.extend_from_slice(b"<hh:slash type=\"NONE\" Crooked=\"0\" isCounter=\"0\"/>");
    out.extend_from_slice(b"<hh:backSlash type=\"NONE\" Crooked=\"0\" isCounter=\"0\"/>");
    let names: [&[u8]; 4] = [
        b"leftBorder",
        b"rightBorder",
        b"topBorder",
        b"bottomBorder",
    ];
    for (i, name) in names.iter().enumerate() {
        emit_border_element(name, &fill.borders[i], out);
    }
    emit_border_element(b"diagonal", &fill.diagonal, out);

    if let Some((r, g, b, _a)) = fill.fill.back_color() {
        let hex = format!("#{r:02X}{g:02X}{b:02X}");
        out.extend_from_slice(b"<hc:fillBrush><hc:winBrush faceColor=\"");
        out.extend_from_slice(hex.as_bytes());
        out.extend_from_slice(b"\" hatchColor=\"#000000\" hatchStyle=\"NONE\" alpha=\"0\"/></hc:fillBrush>");
    }

    out.extend_from_slice(b"</hh:borderFill>");
}

/// Map an IR `Border` to a HWPX `<hh:NAME type=… width=… color=…/>`
/// element. `kind` is HWP5's u8 line-style enum; values >7 fall back
/// to `SOLID`. `width` is in 0.1mm units.
fn emit_border_element(name: &[u8], border: &Border, out: &mut Vec<u8>) {
    let kind_str = match border.kind {
        0 => "NONE",
        1 => "SOLID",
        2 => "DASH",
        3 => "DOT",
        4 => "DASH_DOT",
        5 => "DASH_DOT_DOT",
        6 => "LONG_DASH",
        7 => "DOUBLE",
        _ => "SOLID",
    };
    let width_mm = (border.width.max(1) as f32) / 10.0;
    let r = (border.color & 0xFF) as u8;
    let g = ((border.color >> 8) & 0xFF) as u8;
    let b = ((border.color >> 16) & 0xFF) as u8;
    let color_hex = format!("#{r:02X}{g:02X}{b:02X}");
    out.extend_from_slice(b"<hh:");
    out.extend_from_slice(name);
    out.extend_from_slice(b" type=\"");
    out.extend_from_slice(kind_str.as_bytes());
    out.extend_from_slice(format!("\" width=\"{width_mm:.2} mm\" color=\"").as_bytes());
    out.extend_from_slice(color_hex.as_bytes());
    out.extend_from_slice(b"\"/>");
}

/// Trait-free helper: write `<hh:NAME hangul=… latin=… …/>` for the
/// 7-script attribute children. Generic over any element whose value
/// type Display-prints as the attr value (covers u16 / u8 / i8).
fn emit_script_array<T: std::fmt::Display>(
    elem: &[u8],
    values: &[T; 7],
    out: &mut Vec<u8>,
) {
    out.push(b'<');
    out.extend_from_slice(b"hh:");
    out.extend_from_slice(elem);
    for (slot, value) in SCRIPT_SLOTS.iter().zip(values.iter()) {
        out.push(b' ');
        out.extend_from_slice(slot.as_bytes());
        out.extend_from_slice(b"=\"");
        out.extend_from_slice(value.to_string().as_bytes());
        out.push(b'"');
    }
    out.extend_from_slice(b"/>");
}

/// Build the bytes to splice before `</hh:charPr>` for any presence-
/// driven children the IR wants enabled but the original didn't
/// emit. Inserts `<hh:bold/>` / `<hh:italic/>` directly; for
/// strikeout / underline the original convention always emits
/// `shape="NONE"` when off, so we only need to insert when both the
/// IR is on AND the document never carried the element. Returns an
/// empty Vec when nothing needs inserting (the common case).
fn build_charpr_missing_inserts(
    shape: &CharShape,
    seen_bold: bool,
    seen_italic: bool,
    seen_strikeout: bool,
    seen_underline: bool,
) -> Vec<u8> {
    let mut out = Vec::new();
    if shape.bold() && !seen_bold {
        out.extend_from_slice(b"<hh:bold/>");
    }
    if shape.italic() && !seen_italic {
        out.extend_from_slice(b"<hh:italic/>");
    }
    if shape.strike() && !seen_strikeout {
        let color = color_to_hex(shape.strike_color.unwrap_or(0));
        out.extend_from_slice(b"<hh:strikeout shape=\"SOLID\" color=\"");
        out.extend_from_slice(color.as_bytes());
        out.extend_from_slice(b"\"/>");
    }
    if shape.underline_kind() != 0 && !seen_underline {
        let color = color_to_hex(shape.underline_color);
        out.extend_from_slice(b"<hh:underline shape=\"SOLID\" color=\"");
        out.extend_from_slice(color.as_bytes());
        out.extend_from_slice(b"\"/>");
    }
    out
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
    fn bold_italic_inserted_before_end_when_ir_says_on() {
        // Original lacks `<hh:bold/>` / `<hh:italic/>`. IR has both
        // on. Rewriter should insert just before `</hh:charPr>`.
        let xml = br##"<hh:head xmlns:hh="h">
            <hh:charProperties>
              <hh:charPr id="0" height="1000" textColor="#000000">
                <hh:fontRef hangul="0" latin="0" hanja="0" japanese="0" other="0" symbol="0" user="0"/>
              </hh:charPr>
            </hh:charProperties>
          </hh:head>"##;
        let mut doc = IrDocument::default();
        let mut cs = CharShape::default();
        cs.attr = 0x0000_0003; // bold + italic bits
        doc.doc_info.char_shapes = vec![cs];
        let out = rewrite_header_xml(xml, &doc).expect("rewrite");
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains("<hh:bold/>"), "bold inserted: {s}");
        assert!(s.contains("<hh:italic/>"), "italic inserted: {s}");
        let bold_pos = s.find("<hh:bold/>").unwrap();
        let end_pos = s.find("</hh:charPr>").unwrap();
        assert!(bold_pos < end_pos, "bold insert before End: {s}");
    }

    #[test]
    fn bold_skipped_when_ir_says_off() {
        // Original has `<hh:bold/>`. IR has bold off. Rewriter
        // should drop the event from output.
        let xml = br##"<hh:head xmlns:hh="h">
            <hh:charProperties>
              <hh:charPr id="0" height="1000" textColor="#000000">
                <hh:fontRef hangul="0" latin="0" hanja="0" japanese="0" other="0" symbol="0" user="0"/>
                <hh:bold/>
                <hh:italic/>
              </hh:charPr>
            </hh:charProperties>
          </hh:head>"##;
        let mut doc = IrDocument::default();
        let cs = CharShape::default(); // attr = 0 → bold/italic off
        doc.doc_info.char_shapes = vec![cs];
        let out = rewrite_header_xml(xml, &doc).expect("rewrite");
        let s = std::str::from_utf8(&out).unwrap();
        assert!(!s.contains("<hh:bold/>"), "bold dropped: {s}");
        assert!(!s.contains("<hh:italic/>"), "italic dropped: {s}");
        // charPr / fontRef structure intact.
        assert!(s.contains("<hh:fontRef"), "fontRef preserved: {s}");
        assert!(s.contains("</hh:charPr>"), "End tag preserved: {s}");
    }

    #[test]
    fn bold_italic_unchanged_when_ir_matches_original() {
        // Original has bold (italic absent). IR matches. Output
        // should preserve the original — no extra italic inserted.
        let xml = br##"<hh:head xmlns:hh="h">
            <hh:charProperties>
              <hh:charPr id="0" height="1000" textColor="#000000">
                <hh:fontRef hangul="0" latin="0" hanja="0" japanese="0" other="0" symbol="0" user="0"/>
                <hh:bold/>
              </hh:charPr>
            </hh:charProperties>
          </hh:head>"##;
        let mut doc = IrDocument::default();
        let mut cs = CharShape::default();
        cs.attr = 0x0000_0002; // bold only
        doc.doc_info.char_shapes = vec![cs];
        let out = rewrite_header_xml(xml, &doc).expect("rewrite");
        let s = std::str::from_utf8(&out).unwrap();
        assert_eq!(s.matches("<hh:bold/>").count(), 1, "bold once: {s}");
        assert!(!s.contains("<hh:italic/>"), "no spurious italic: {s}");
    }

    #[test]
    fn missing_strikeout_inserted_when_ir_strike_on() {
        // Original lacks `<hh:strikeout/>` element entirely (rare —
        // most HWPX always emits it). IR has strike=true. Insert.
        let xml = br##"<hh:head xmlns:hh="h">
            <hh:charProperties>
              <hh:charPr id="0" height="1000" textColor="#000000">
                <hh:fontRef hangul="0" latin="0" hanja="0" japanese="0" other="0" symbol="0" user="0"/>
              </hh:charPr>
            </hh:charProperties>
          </hh:head>"##;
        let mut doc = IrDocument::default();
        let mut cs = CharShape::default();
        cs.attr = 1 << 21; // strike bit
        cs.strike_color = Some(0x0000_00FF); // R=FF
        doc.doc_info.char_shapes = vec![cs];
        let out = rewrite_header_xml(xml, &doc).expect("rewrite");
        let s = std::str::from_utf8(&out).unwrap();
        assert!(
            s.contains(r##"<hh:strikeout shape="SOLID" color="#FF0000"/>"##),
            "strikeout inserted: {s}"
        );
    }

    #[test]
    fn fontref_ratio_relsz_arrays_overlaid() {
        let xml = br##"<hh:head xmlns:hh="h">
            <hh:charProperties>
              <hh:charPr id="0" height="1000" textColor="#000000">
                <hh:fontRef hangul="0" latin="0" hanja="0" japanese="0" other="0" symbol="0" user="0"/>
                <hh:ratio hangul="100" latin="100" hanja="100" japanese="100" other="100" symbol="100" user="100"/>
                <hh:relSz hangul="100" latin="100" hanja="100" japanese="100" other="100" symbol="100" user="100"/>
              </hh:charPr>
            </hh:charProperties>
          </hh:head>"##;
        let mut doc = IrDocument::default();
        let mut cs = CharShape::default();
        cs.font_ids = [3, 5, 0, 0, 0, 0, 0];
        cs.ratios = [120, 90, 100, 100, 100, 100, 100];
        cs.rel_sizes = [80, 110, 100, 100, 100, 100, 100];
        doc.doc_info.char_shapes = vec![cs];
        let out = rewrite_header_xml(xml, &doc).expect("rewrite");
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains(r#"<hh:fontRef hangul="3" latin="5""#), "fontRef hangul/latin overlaid: {s}");
        assert!(s.contains(r#"<hh:ratio hangul="120" latin="90""#), "ratio overlaid: {s}");
        assert!(s.contains(r#"<hh:relSz hangul="80" latin="110""#), "relSz overlaid: {s}");
        assert!(s.contains(r#"hanja="0""#), "hanja zero preserved on fontRef: {s}");
        assert!(s.contains(r#"hanja="100""#), "hanja default preserved on ratio/relSz: {s}");
    }

    #[test]
    fn spacing_offset_signed_arrays_overlaid() {
        let xml = br##"<hh:head xmlns:hh="h">
            <hh:charProperties>
              <hh:charPr id="0" height="1000" textColor="#000000">
                <hh:fontRef hangul="0" latin="0" hanja="0" japanese="0" other="0" symbol="0" user="0"/>
                <hh:spacing hangul="0" latin="0" hanja="0" japanese="0" other="0" symbol="0" user="0"/>
                <hh:offset hangul="0" latin="0" hanja="0" japanese="0" other="0" symbol="0" user="0"/>
              </hh:charPr>
            </hh:charProperties>
          </hh:head>"##;
        let mut doc = IrDocument::default();
        let mut cs = CharShape::default();
        cs.char_spacings = [-5, 10, 0, 0, 0, 0, 0];
        cs.char_offsets = [3, -7, 0, 0, 0, 0, 0];
        doc.doc_info.char_shapes = vec![cs];
        let out = rewrite_header_xml(xml, &doc).expect("rewrite");
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains(r#"<hh:spacing hangul="-5" latin="10""#), "negative spacing overlaid: {s}");
        assert!(s.contains(r#"<hh:offset hangul="3" latin="-7""#), "negative offset overlaid: {s}");
    }

    #[test]
    fn array_children_outside_charpr_left_alone() {
        // A `<hh:fontRef>` outside charPr context should NOT be
        // rewritten — only charPr's child overlays run.
        let xml = br##"<hh:head xmlns:hh="h">
            <hh:bogusOuter>
              <hh:fontRef hangul="0" latin="0" hanja="0" japanese="0" other="0" symbol="0" user="0"/>
            </hh:bogusOuter>
          </hh:head>"##;
        let mut doc = IrDocument::default();
        let mut cs = CharShape::default();
        cs.font_ids = [99, 99, 99, 99, 99, 99, 99];
        doc.doc_info.char_shapes = vec![cs];
        let out = rewrite_header_xml(xml, &doc).expect("rewrite");
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains(r#"hangul="0""#), "outside charPr context kept: {s}");
        assert!(!s.contains(r#"hangul="99""#), "should not overlay outside charPr: {s}");
    }

    #[test]
    fn out_of_range_para_pr_is_dropped_and_ir_id_inserted() {
        // paraPr id=5 in original, IR has only 1 paraShape (id=0).
        // New semantic: drop the out-of-range span (user removed it
        // from IR by truncate) and insert a fresh paraPr id=0
        // (user added it since the original lacked id=0).
        let xml = br##"<hh:head xmlns:hh="h">
            <hh:paraProperties>
              <hh:paraPr id="5">
                <hh:align horizontal="LEFT"/>
              </hh:paraPr>
            </hh:paraProperties>
          </hh:head>"##;
        let doc = doc_with_para_shapes(vec![ps_with_align(2)]); // RIGHT
        let out = rewrite_header_xml(xml, &doc).expect("rewrite");
        let s = std::str::from_utf8(&out).unwrap();
        assert!(!s.contains(r#"id="5""#), "id=5 dropped: {s}");
        assert!(!s.contains(r#"horizontal="LEFT""#), "old LEFT dropped: {s}");
        // The fresh paraPr emitted for an IR-side shape now carries
        // the full Hancom-style child set; assert on the parts that
        // identify the right id+align without pinning the exact
        // attribute order.
        assert!(s.contains(r#"<hh:paraPr id="0""#), "id=0 inserted: {s}");
        assert!(s.contains(r#"horizontal="RIGHT""#), "IR's align applied: {s}");
    }

    #[test]
    fn out_of_range_para_pr_with_no_ir_drops_silently() {
        // No IR para shapes at all + an out-of-range paraPr → just
        // drops the span without inserting anything.
        let xml = br##"<hh:head xmlns:hh="h">
            <hh:paraProperties>
              <hh:paraPr id="2">
                <hh:align horizontal="LEFT"/>
              </hh:paraPr>
            </hh:paraProperties>
          </hh:head>"##;
        let doc = doc_with_para_shapes(vec![]);
        let out = rewrite_header_xml(xml, &doc).expect("rewrite");
        let s = std::str::from_utf8(&out).unwrap();
        assert!(!s.contains(r#"id="2""#), "paraPr id=2 dropped: {s}");
        assert!(!s.contains("horizontal=\"LEFT\""), "align child gone: {s}");
        assert!(s.contains("<hh:paraProperties>"), "container kept: {s}");
        assert!(s.contains("</hh:paraProperties>"), "container kept: {s}");
    }

    #[test]
    fn ir_added_para_shape_inserted_before_container_end() {
        // Original has paraPr id=0. IR has 2 paraShapes (one push).
        // Expect id=0 overlaid with IR's align; id=1 inserted as a
        // brand-new block just before `</hh:paraProperties>`.
        let xml = br##"<hh:head xmlns:hh="h">
            <hh:paraProperties>
              <hh:paraPr id="0">
                <hh:align horizontal="LEFT"/>
              </hh:paraPr>
            </hh:paraProperties>
          </hh:head>"##;
        let doc = doc_with_para_shapes(vec![
            ps_with_align(2), // RIGHT
            ps_with_align(3), // CENTER
        ]);
        let out = rewrite_header_xml(xml, &doc).expect("rewrite");
        let s = std::str::from_utf8(&out).unwrap();
        assert!(
            s.contains(r#"<hh:paraPr id="0">"#),
            "original paraPr 0 kept: {s}"
        );
        assert!(s.contains(r#"horizontal="RIGHT""#), "id=0 overlaid: {s}");
        // id=1 is freshly emitted by the rewriter — assert id+align
        // independently rather than pinning the exact tag order, since
        // the fresh-paraPr template now carries the Hancom-typical
        // attribute set (tabPrIDRef, lineSpacing, breakSetting, …).
        assert!(s.contains(r#"<hh:paraPr id="1""#), "id=1 inserted: {s}");
        assert!(s.contains(r#"horizontal="CENTER""#), "id=1 align applied: {s}");
    }

    #[test]
    fn ir_added_char_shape_inserted_before_container_end() {
        let xml = br##"<hh:head xmlns:hh="h">
            <hh:charProperties>
              <hh:charPr id="0" height="1000" textColor="#000000">
                <hh:fontRef hangul="0" latin="0" hanja="0" japanese="0" other="0" symbol="0" user="0"/>
              </hh:charPr>
            </hh:charProperties>
          </hh:head>"##;
        let mut doc = IrDocument::default();
        let mut cs0 = CharShape::default();
        cs0.base_size = 1000;
        let mut cs1 = CharShape::default();
        cs1.base_size = 1500;
        cs1.color = 0x0000_00FF; // R=FF
        cs1.attr = 0x0000_0002; // bold
        doc.doc_info.char_shapes = vec![cs0, cs1];
        let out = rewrite_header_xml(xml, &doc).expect("rewrite");
        let s = std::str::from_utf8(&out).unwrap();
        assert!(
            s.contains(r##"<hh:charPr id="1" height="1500" textColor="#FF0000""##),
            "new charPr id=1 inserted: {s}"
        );
        assert!(
            s.contains("<hh:bold/>"),
            "new charPr emits bold from IR: {s}"
        );
        let id1_pos = s.find(r#"id="1""#).unwrap();
        let end_pos = s.find("</hh:charProperties>").unwrap();
        assert!(id1_pos < end_pos, "insert before container End: {s}");
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
