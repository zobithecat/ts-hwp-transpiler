//! DocInfo synthesis shared by every Markdown import path.
//!
//! Both [`super::markdown::from_gfm_markdown`] and
//! [`super::markdown_llm::from_llm_markdown`] need the same IR-side
//! style table for headings to round-trip back into a HWPX:
//!
//!   * `paraShapes[0..=6]` with `heading_level` bits encoded in
//!     `attribute >> 24`.
//!   * `charShapes[0..=6]` — body 10 pt, H1 20 pt-bold, H2 17 pt-bold,
//!     down to H6 11 pt-bold — so HWPX viewers visually distinguish
//!     headings from body text.
//!   * `styles[0..=6]` named `"본문"` / `"개요 1"` … `"개요 6"` so the
//!     existing human-Markdown exporter's `style.name`-keyed
//!     `heading_level` lookup re-classifies headings on the symmetric
//!     direction.
//!
//! The function is idempotent: it overwrites whatever was in the
//! three slots, so callers building a fresh IR don't have to clear
//! first.

use hwp_transpiler_core::ir::{CharShape, IrDocument, ParaShape, Style};

pub fn populate_heading_doc_info(doc: &mut IrDocument) {
    doc.doc_info.para_shapes = synthesise_heading_para_shapes();
    doc.doc_info.char_shapes = synthesise_heading_char_shapes();
    doc.doc_info.styles = synthesise_heading_styles();
}

/// `[default body, H1 … H6]` — index 0 is body, 1..=6 carry
/// `heading_level` in bits 24..=26 of `ParaShape::attribute`.
pub fn synthesise_heading_para_shapes() -> Vec<ParaShape> {
    let mut shapes = vec![ParaShape::default()];
    for level in 1u8..=6 {
        let mut p = ParaShape::default();
        p.attribute = (level as u32) << 24;
        shapes.push(p);
    }
    shapes
}

/// `[body 10pt, H1 20pt-bold, …, H6 11pt-bold]` so HWPX viewers
/// render headings visibly distinct from body. Sizes are in 1/100 pt.
pub fn synthesise_heading_char_shapes() -> Vec<CharShape> {
    // 장평(ratios) / 상대크기(rel_sizes) are percentages — the struct
    // default of 0 means "0% wide / 0% tall" to strict viewers (HWP
    // 2014 collapses the whole document into a dot), so every
    // synthesised shape carries the 100% neutral value explicitly.
    let base = CharShape {
        ratios: [100; 7],
        rel_sizes: [100; 7],
        ..CharShape::default()
    };
    let mut shapes = vec![{
        let mut cs = base.clone();
        cs.base_size = 1000;
        cs
    }];
    let sizes = [2000, 1700, 1500, 1300, 1200, 1100];
    for size in sizes {
        let mut cs = base.clone();
        cs.base_size = size;
        cs.attr = 0x0000_0002; // bold
        shapes.push(cs);
    }
    shapes
}

/// `["본문", "개요 1", … "개요 6"]` — each heading style references
/// the matching `paraShape` and `charShape` index so the synthesised
/// font-size / bold defaults reach rendered output.
pub fn synthesise_heading_styles() -> Vec<Style> {
    let mut styles = vec![Style {
        name: "본문".to_string(),
        english_name: "Body".to_string(),
        para_shape_id: 0,
        char_shape_id: 0,
        ..Style::default()
    }];
    for level in 1u8..=6 {
        styles.push(Style {
            name: format!("개요 {level}"),
            english_name: format!("Outline {level}"),
            para_shape_id: level as u16,
            char_shape_id: level as u16,
            ..Style::default()
        });
    }
    styles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn populates_seven_slot_heading_table() {
        let mut doc = IrDocument::default();
        populate_heading_doc_info(&mut doc);
        assert_eq!(doc.doc_info.para_shapes.len(), 7);
        assert_eq!(doc.doc_info.char_shapes.len(), 7);
        assert_eq!(doc.doc_info.styles.len(), 7);
    }

    #[test]
    fn heading_level_bits_match_index() {
        let shapes = synthesise_heading_para_shapes();
        assert_eq!(shapes[0].heading_level(), 0);
        assert_eq!(shapes[1].heading_level(), 1);
        assert_eq!(shapes[6].heading_level(), 6);
    }

    #[test]
    fn heading_char_shapes_progressively_larger_than_body() {
        let shapes = synthesise_heading_char_shapes();
        assert_eq!(shapes[0].base_size, 1000); // body
        assert!(shapes[1].base_size > shapes[2].base_size);
        assert!(shapes[6].base_size > shapes[0].base_size);
        for s in shapes.iter().skip(1) {
            assert!(s.bold(), "headings should be bold");
        }
    }

    #[test]
    fn char_shapes_carry_neutral_percent_ratios() {
        // ratio/relSz 0 renders as 0%-sized glyphs in HWP 2014 — every
        // synthesised shape must ship the 100% neutral explicitly.
        for (i, s) in synthesise_heading_char_shapes().iter().enumerate() {
            assert_eq!(s.ratios, [100; 7], "shape {i} ratios");
            assert_eq!(s.rel_sizes, [100; 7], "shape {i} rel_sizes");
        }
    }

    #[test]
    fn styles_use_korean_outline_naming_for_export_lookup() {
        let styles = synthesise_heading_styles();
        assert_eq!(styles[0].name, "본문");
        assert_eq!(styles[1].name, "개요 1");
        assert_eq!(styles[6].name, "개요 6");
    }

    #[test]
    fn styles_index_matches_para_and_char_shape_refs() {
        let styles = synthesise_heading_styles();
        for (idx, s) in styles.iter().enumerate() {
            assert_eq!(s.para_shape_id as usize, idx);
            assert_eq!(s.char_shape_id as usize, idx);
        }
    }
}
