//! Inline-asset round-trip via the LLM emit / re-parse path.
//! Builds a synthetic IR with one PNG bin entry + a wrapper
//! Paragraph that hosts a `PictureControl` referencing it, exports
//! through `to_llm_markdown` with `AssetMode::Inline`, then re-imports
//! via `from_llm_markdown` and confirms the picture + the binary
//! bytes both made the round trip.

use hwp_transpiler_codec::export::markdown::{
    to_markdown_with, AssetMode, LlmOptions, MdOptions,
};
use hwp_transpiler_codec::export::markdown_llm::to_llm_markdown;
use hwp_transpiler_codec::import::markdown::from_markdown;
use hwp_transpiler_codec::import::markdown_llm::from_llm_markdown;
use hwp_transpiler_core::ir::{
    BinaryEntry, Control, ControlKind, IrDocument, Paragraph, PictureControl, Section,
};
use std::io::Cursor;

fn solid_png(w: u32, h: u32) -> Vec<u8> {
    let img = image::DynamicImage::ImageRgb8(image::RgbImage::new(w, h));
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

fn doc_with_one_picture() -> IrDocument {
    let mut doc = IrDocument::default();
    doc.bin_data.push(BinaryEntry {
        id: "image1.png".into(),
        mime: Some("image/png".into()),
        bytes: solid_png(40, 20),
    });
    let mut wrapper = Paragraph::default();
    wrapper.text = "\u{FFFC}".into();
    wrapper.controls.push(Control {
        kind: ControlKind::Picture(PictureControl {
            bin_id: 1,
            // 40 px @ 72 dpi ≈ 14 mm — round-tripped via mm.
            width_hwpu: ((40.0_f64) * 7200.0 / 72.0).round() as u32,
            height_hwpu: ((20.0_f64) * 7200.0 / 72.0).round() as u32,
        }),
        caption_text: None,
    });
    doc.sections.push(Section {
        paragraphs: vec![wrapper],
        ..Section::default()
    });
    doc
}

fn opts_with_inline_assets() -> MdOptions {
    MdOptions {
        llm: Some(LlmOptions::default()),
        asset_mode: AssetMode::Inline,
        asset_dpi: Some(72),
        ..MdOptions::default()
    }
}

#[test]
fn inline_export_includes_figure_and_asset_records() {
    let doc = doc_with_one_picture();
    let md = to_llm_markdown(&doc, &opts_with_inline_assets());

    assert!(md.contains("FIGURE[id=fig-1,bin_id=1"), "FIGURE record: {md}");
    assert!(md.contains("<!-- hwp-transpiler: assets -->"), "footer: {md}");
    assert!(
        md.contains("ASSET[id=asset-1,bin_id=1,mime=image/png"),
        "ASSET record: {md}",
    );
    assert!(md.contains("DATA: data:image/png;base64,"), "DATA URI: {md}");
}

#[test]
fn inline_import_restores_bin_data_and_picture_control() {
    let doc = doc_with_one_picture();
    let md = to_llm_markdown(&doc, &opts_with_inline_assets());

    let reloaded = from_llm_markdown(&md).expect("import");

    // Picture wrapper paragraph is reconstructed.
    let pic = reloaded.sections.iter().flat_map(|s| &s.paragraphs).find_map(|p| {
        p.controls.iter().find_map(|c| match &c.kind {
            ControlKind::Picture(pic) => Some(pic),
            _ => None,
        })
    });
    let pic = pic.expect("picture control survived round-trip");
    assert_eq!(pic.bin_id, 1, "bin_id linked");
    assert!(pic.width_hwpu > 0, "width carried via mm");
    assert!(pic.height_hwpu > 0);

    // Asset bytes flowed back into bin_data via the data URI decode.
    assert_eq!(reloaded.bin_data.len(), 1);
    let entry = &reloaded.bin_data[0];
    assert_eq!(entry.id, "image1.png");
    assert!(!entry.bytes.is_empty(), "bin bytes restored");
}

#[test]
fn inline_skips_when_no_pictures_present() {
    let mut doc = IrDocument::default();
    doc.sections.push(Section {
        paragraphs: vec![{
            let mut p = Paragraph::default();
            p.text = "본문".into();
            p
        }],
        ..Section::default()
    });
    let md = to_llm_markdown(&doc, &opts_with_inline_assets());
    assert!(
        !md.contains("<!-- hwp-transpiler: assets -->"),
        "no footer when bin_data empty: {md}"
    );
}

#[test]
fn gfm_inline_image_round_trips_via_data_uri() {
    let doc = doc_with_one_picture();
    let opts = MdOptions {
        asset_mode: AssetMode::Inline,
        asset_dpi: Some(72),
        ..MdOptions::default()
    };
    let md = to_markdown_with(&doc, &opts);
    assert!(
        md.contains("![](data:image/png;base64,"),
        "human-mode inline emits data URI: {md}"
    );
    let reloaded = from_markdown(&md).expect("import");
    let pic = reloaded
        .sections
        .iter()
        .flat_map(|s| &s.paragraphs)
        .find_map(|p| {
            p.controls.iter().find_map(|c| match &c.kind {
                ControlKind::Picture(pic) => Some(pic),
                _ => None,
            })
        })
        .expect("PictureControl restored on GFM import");
    assert!(pic.bin_id > 0, "synthesised bin_id assigned");
    assert_eq!(reloaded.bin_data.len(), 1, "data URI decoded into bin_data");
}

#[test]
fn section_bytes_round_trip_via_inline_footer() {
    // Build a doc with one section that carries verbatim XML bytes
    // — simulating what the HWPX reader stores when it loads a real
    // file. After exporting to LLM Markdown with inline assets and
    // re-importing, the section's `stream_bytes` should be back.
    let mut doc = IrDocument::default();
    let mut section = Section::default();
    section.paragraphs.push({
        let mut p = Paragraph::default();
        p.text = "본문".into();
        p
    });
    // The frozen XML's text must match the typed body ("본문") —
    // the import-side verify-gate compares the two and only keeps
    // the verbatim cache when the body wasn't edited after export.
    let frozen: &[u8] = b"<hs:sec><hp:p><hp:run><hp:t>\xEB\xB3\xB8\xEB\xAC\xB8</hp:t></hp:run></hp:p></hs:sec>";
    section.stream_bytes = Some(frozen.to_vec());
    doc.sections.push(section);

    let opts = MdOptions {
        llm: Some(LlmOptions::default()),
        asset_mode: AssetMode::Inline,
        asset_dpi: None,
        ..MdOptions::default()
    };
    let md = to_llm_markdown(&doc, &opts);
    assert!(
        md.contains(&format!("SECTION_BYTES[id=section-0,len={}]", frozen.len())),
        "SECTION_BYTES record emitted: {md}"
    );

    let reloaded = from_llm_markdown(&md).expect("import");
    assert_eq!(reloaded.sections.len(), 1);
    let bytes = reloaded.sections[0]
        .stream_bytes
        .as_ref()
        .expect("section stream_bytes restored");
    assert_eq!(
        bytes.as_slice(),
        frozen,
        "section bytes round-trip byte-equal"
    );
}

#[test]
fn edited_body_invalidates_section_bytes_on_import() {
    // Same transport as above, but the body md is edited between
    // export and import. Replaying the frozen bytes would silently
    // discard the edit, so the verify-gate must drop `stream_bytes`
    // and let the writer rebuild the section from the edited
    // paragraphs.
    let mut doc = IrDocument::default();
    let mut section = Section::default();
    section.paragraphs.push({
        let mut p = Paragraph::default();
        p.text = "본문".into();
        p
    });
    section.stream_bytes = Some(
        b"<hs:sec><hp:p><hp:run><hp:t>\xEB\xB3\xB8\xEB\xAC\xB8</hp:t></hp:run></hp:p></hs:sec>"
            .to_vec(),
    );
    doc.sections.push(section);

    let opts = MdOptions {
        llm: Some(LlmOptions::default()),
        asset_mode: AssetMode::Inline,
        asset_dpi: None,
        ..MdOptions::default()
    };
    let md = to_llm_markdown(&doc, &opts);
    let edited = md.replace("TEXT: 본문", "TEXT: 본문 (AI가 고침)");
    assert_ne!(md, edited, "edit applied to the body record");

    let reloaded = from_llm_markdown(&edited).expect("import");
    assert!(
        reloaded.sections[0].stream_bytes.is_none(),
        "verify-gate drops stale frozen bytes so the edit survives"
    );
    assert_eq!(reloaded.sections[0].paragraphs[0].text, "본문 (AI가 고침)");
}

#[test]
fn asset_mode_none_omits_footer_even_with_pictures() {
    let doc = doc_with_one_picture();
    let opts = MdOptions {
        llm: Some(LlmOptions::default()),
        asset_mode: AssetMode::None,
        asset_dpi: Some(72),
        ..MdOptions::default()
    };
    let md = to_llm_markdown(&doc, &opts);
    assert!(
        !md.contains("<!-- hwp-transpiler: assets -->"),
        "AssetMode::None omits footer: {md}"
    );
    // FIGURE record still emitted (it's metadata-only without DATA).
    assert!(md.contains("FIGURE["));
}
