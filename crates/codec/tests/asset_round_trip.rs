//! Inline-asset round-trip via the LLM emit / re-parse path.
//! Builds a synthetic IR with one PNG bin entry + a wrapper
//! Paragraph that hosts a `PictureControl` referencing it, exports
//! through `to_llm_markdown` with `AssetMode::Inline`, then re-imports
//! via `from_llm_markdown` and confirms the picture + the binary
//! bytes both made the round trip.

use hwp_transpiler_codec::export::markdown::{AssetMode, LlmOptions, MdOptions};
use hwp_transpiler_codec::export::markdown_llm::to_llm_markdown;
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
