//! Shared asset-footer renderer used by both Markdown emit paths
//! (LLM record format and human-readable CommonMark).
//!
//! Output shape — same on both sides so the importer can ride one
//! parser:
//!
//! ```text
//! <!-- hwp-transpiler: assets -->
//!
//! ASSET[id=asset-1,bin_id=1,mime=image/png,width=283,height=189,dpi=72]
//! DATA: data:image/png;base64,iVBORw0K…
//!
//! ASSET[id=asset-2,…]
//! DATA: …
//! ```
//!
//! When `MdOptions::asset_mode == None`, the renderers don't call
//! into here at all and the footer never appears.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use hwp_transpiler_core::ir::IrDocument;

use crate::asset_pipeline::{encode_for_md, EncodeOpts};

use super::markdown::MdOptions;

/// Stamped above the table so the importer can locate where the
/// asset registry begins. Mirrors the format-dispatch comment.
pub const FORMAT_HEADER_ASSETS: &str = "<!-- hwp-transpiler: assets -->";

/// Build the footer block: encoded picture assets plus
/// per-section verbatim XML bytes (base64). Empty string when
/// `doc.bin_data` is empty *and* no `Section::stream_bytes` cache
/// exists — that lets the caller emit unconditionally without
/// ending up with a stranded marker.
///
/// Section bytes ride in the same footer because they share the
/// same "binary metadata, drop into a separate companion in split
/// mode so it doesn't bloat the LLM context" property — keeping
/// them in one place means a single parser handles both.
pub fn render_assets_block(doc: &IrDocument, opts: &MdOptions) -> String {
    let encoded = encode_for_md(
        &doc.bin_data,
        &EncodeOpts {
            dpi: opts.asset_dpi,
        },
    );
    let has_section_bytes = doc
        .sections
        .iter()
        .any(|s| s.stream_bytes.as_ref().map(|b| !b.is_empty()).unwrap_or(false));
    if encoded.is_empty() && !has_section_bytes {
        return String::new();
    }
    let mut out = String::new();
    out.push_str(FORMAT_HEADER_ASSETS);
    out.push('\n');
    out.push('\n');
    for asset in &encoded {
        let bin = asset
            .bin_id
            .map(|n| n.to_string())
            .unwrap_or_else(|| asset.source_id.clone());
        let dims = match (asset.width, asset.height) {
            (Some(w), Some(h)) => format!(",width={w},height={h}"),
            _ => String::new(),
        };
        let dpi_attr = match opts.asset_dpi {
            Some(n) => format!(",dpi={n}"),
            None => String::new(),
        };
        out.push_str(&format!(
            "ASSET[id={id},bin_id={bin},mime={mime}{dims}{dpi_attr},source_id={src}]\n",
            id = asset.asset_id,
            mime = asset.mime,
            src = asset.source_id,
        ));
        out.push_str("DATA: ");
        out.push_str(&asset.data_uri);
        out.push_str("\n\n");
    }
    // Per-section verbatim XML bytes. Round-trip through MD then
    // back through the writer reproduces the original section
    // byte-equal (modulo any IR-side mutation that's already
    // invalidated `stream_bytes` upstream). Critical for viewers
    // that depend on `<hp:linesegarray>` / `<hp:secPr>` /
    // paragraph layout metadata the typed parser doesn't decode.
    for (idx, section) in doc.sections.iter().enumerate() {
        let Some(bytes) = section.stream_bytes.as_ref() else {
            continue;
        };
        if bytes.is_empty() {
            continue;
        }
        out.push_str(&format!(
            "SECTION_BYTES[id=section-{idx},len={len}]\n",
            len = bytes.len(),
        ));
        out.push_str("DATA: data:application/octet-stream;base64,");
        out.push_str(&STANDARD.encode(bytes));
        out.push_str("\n\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use hwp_transpiler_core::ir::BinaryEntry;
    use std::io::Cursor;

    fn solid_png(w: u32, h: u32) -> Vec<u8> {
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::new(w, h));
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    fn doc_with_image() -> IrDocument {
        let mut doc = IrDocument::default();
        doc.bin_data.push(BinaryEntry {
            id: "image1.png".into(),
            mime: Some("image/png".into()),
            bytes: solid_png(40, 20),
        });
        doc
    }

    #[test]
    fn empty_doc_emits_empty_string() {
        let doc = IrDocument::default();
        let opts = MdOptions {
            asset_dpi: Some(72),
            ..MdOptions::default()
        };
        assert_eq!(render_assets_block(&doc, &opts), "");
    }

    #[test]
    fn emits_marker_and_one_asset_block() {
        let doc = doc_with_image();
        let opts = MdOptions {
            asset_dpi: Some(72),
            ..MdOptions::default()
        };
        let body = render_assets_block(&doc, &opts);
        assert!(body.contains(FORMAT_HEADER_ASSETS));
        assert!(body.contains("ASSET[id=asset-1,bin_id=1,mime=image/png"));
        assert!(body.contains("DATA: data:image/png;base64,"));
        assert!(body.contains("source_id=image1.png"));
    }

    #[test]
    fn dpi_attr_is_emitted_when_set() {
        let doc = doc_with_image();
        let opts = MdOptions {
            asset_dpi: Some(36),
            ..MdOptions::default()
        };
        let body = render_assets_block(&doc, &opts);
        assert!(body.contains(",dpi=36"));
    }

    #[test]
    fn dpi_omitted_when_none() {
        let doc = doc_with_image();
        let opts = MdOptions {
            asset_dpi: None,
            ..MdOptions::default()
        };
        let body = render_assets_block(&doc, &opts);
        assert!(!body.contains(",dpi="));
    }
}
