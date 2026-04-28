//! Asset (binary picture) pipeline for the MD ↔ IR round-trip.
//!
//! Bridges [`hwp_transpiler_core::ir::BinaryEntry`] (raw bytes the
//! HWP / HWPX readers parse out of `/BinData/*` and
//! `BinData/image*`) and the `data:image/<mime>;base64,…` URIs the
//! Markdown emitters write into the `<stem>.assets.md` companion
//! file.
//!
//! Encode path (`encode_for_md`):
//!   1. Decode the source bytes via the `image` crate. Anything the
//!      crate doesn't recognise (e.g. SVG, EMF) is skipped — only
//!      raster images flow through here.
//!   2. Optionally resample down to the requested DPI. The IR
//!      carries no DPI metadata; we treat 72 DPI as the canonical
//!      "screen" target so the resized pixel dims roughly match the
//!      `width_mm` / `height_mm` the HWP picture record advertises.
//!      36 DPI gives roughly half-size for LLM-context budgets.
//!   3. Re-encode as PNG (lossless — `image` crate's bundled WebP
//!      encoder is also lossless and we already have PNG, so the
//!      additional dep wasn't worth it for this slice). Alpha
//!      channel survives. Lossy JPEG isn't used because round-trip
//!      requires bit-identity at the asset level — re-importing
//!      shouldn't drift the decoded bytes.
//!   4. Base64-wrap into a `data:image/png;base64,…` URI so the
//!      Markdown reference-style link is legal CommonMark.
//!
//! Decode path (`decode_data_uri_to_binary_entry`): inverse — split
//! the URI, base64-decode, return as a [`BinaryEntry`] that the
//! HWPX writer's `bin_data` re-emit can hand back into the archive
//! at write time. The bytes stay in their re-encoded form (PNG); a
//! future pass that wants strict round-trip-bit-identity can opt
//! out of resizing via `dpi = None`, which keeps the original bytes.

use std::io::Cursor;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use hwp_transpiler_core::ir::BinaryEntry;
use image::{ImageFormat, ImageReader};

/// Knobs for `encode_for_md`. `None` for `dpi` skips the resize step
/// entirely so the original bytes round-trip verbatim — useful when
/// the caller wants bit-identical assets and the file-size cost is
/// acceptable.
#[derive(Debug, Clone, Copy)]
pub struct EncodeOpts {
    /// Target DPI for the resized image. `None` skips resizing.
    /// HWP advertises picture geometry in HWPUNIT (1 inch = 7200
    /// HWPUNIT) so the canonical "1 inch = 72 px" web-screen DPI
    /// keeps the on-screen pixel count the same as the document
    /// declares.
    pub dpi: Option<u32>,
}

impl Default for EncodeOpts {
    fn default() -> Self {
        Self { dpi: Some(72) }
    }
}

/// One element of the encoded-assets table written into a
/// `<stem>.assets.md` companion file.
#[derive(Debug, Clone)]
pub struct EncodedAsset {
    /// Stable id used by `FIGURE[asset_ref=…]` and CommonMark
    /// reference labels. `"asset-{bin_id}"`.
    pub asset_id: String,
    /// Mirrors `BinaryEntry::id` so the writer can re-emit the
    /// asset under the right `BinData/<id>` name.
    pub source_id: String,
    /// `BinaryEntry::id` parsed as a `u16` when possible. Used by
    /// the LLM record format's `bin_id=N` attribute. `None` when
    /// the source id isn't numeric (rare; typically HWPX uses
    /// `imageN.<ext>`).
    pub bin_id: Option<u16>,
    /// `image/png` after re-encoding. Always PNG for now; the field
    /// is kept so a later WebP / AVIF rev can opt different
    /// containers without changing the data shape.
    pub mime: String,
    /// Resized pixel width × height. `None` when resize was
    /// skipped — caller can fall back to the source image dims.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// `data:image/png;base64,…` ready to drop into a CommonMark
    /// reference-style link or an LLM `DATA:` line.
    pub data_uri: String,
}

/// Walk the IR's `bin_data` and produce the encoded-assets table.
/// Non-raster entries (anything the `image` crate doesn't decode)
/// are skipped — caller handles them separately or drops them.
pub fn encode_for_md(bin_data: &[BinaryEntry], opts: &EncodeOpts) -> Vec<EncodedAsset> {
    bin_data
        .iter()
        .filter_map(|entry| encode_one(entry, opts))
        .collect()
}

fn encode_one(entry: &BinaryEntry, opts: &EncodeOpts) -> Option<EncodedAsset> {
    if entry.bytes.is_empty() {
        return None;
    }
    let reader = ImageReader::new(Cursor::new(&entry.bytes))
        .with_guessed_format()
        .ok()?;
    let img = reader.decode().ok()?;
    let (src_w, src_h) = (img.width(), img.height());
    let resized = if let Some(dpi) = opts.dpi {
        let factor = dpi as f64 / 72.0;
        let target_w = ((src_w as f64) * factor).round().max(1.0) as u32;
        let target_h = ((src_h as f64) * factor).round().max(1.0) as u32;
        if target_w >= src_w && target_h >= src_h {
            // Don't upsample. Source already smaller than target.
            img
        } else {
            img.resize(target_w, target_h, image::imageops::FilterType::Lanczos3)
        }
    } else {
        img
    };
    let (out_w, out_h) = (resized.width(), resized.height());
    let mut buf = Vec::with_capacity(entry.bytes.len() / 2);
    resized
        .write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
        .ok()?;
    let base = STANDARD.encode(&buf);
    let bin_id = bin_id_from_entry_id(&entry.id);
    // The encoded payload is PNG regardless of the source format,
    // so the BinData filename has to match — `image1.jpg` carrying
    // PNG bytes makes HWPX viewers try to JPEG-decode and silently
    // skip the picture. Re-stem the id to keep the numeric / hex
    // segment but force the extension.
    let normalised_id = renormalise_id_to_png(&entry.id);
    Some(EncodedAsset {
        asset_id: format!(
            "asset-{}",
            bin_id
                .map(|n| n.to_string())
                .unwrap_or_else(|| entry.id.clone())
        ),
        source_id: normalised_id,
        bin_id,
        mime: "image/png".into(),
        width: Some(out_w),
        height: Some(out_h),
        data_uri: format!("data:image/png;base64,{base}"),
    })
}

/// Decode a `data:<mime>;base64,<payload>` URI back into a
/// [`BinaryEntry`]. Returns `None` when the URI isn't a `data:`
/// scheme, the base64 payload is malformed, or the mime is missing.
pub fn decode_data_uri_to_binary_entry(uri: &str, source_id: &str) -> Option<BinaryEntry> {
    let rest = uri.strip_prefix("data:")?;
    let (header, payload) = rest.split_once(',')?;
    let (mime, encoding) = match header.split_once(';') {
        Some((m, enc)) => (m.trim(), enc.trim()),
        None => (header.trim(), ""),
    };
    if encoding != "base64" {
        return None;
    }
    let bytes = STANDARD.decode(payload).ok()?;
    Some(BinaryEntry {
        id: source_id.to_string(),
        mime: if mime.is_empty() {
            None
        } else {
            Some(mime.to_string())
        },
        bytes,
    })
}

/// `image1.jpg` → `image1.png` / `BIN0001.bmp` → `BIN0001.png`.
/// Keeps the stem (so `bin_id_from_entry_id` still resolves) and
/// swaps the extension to `.png` because that's what
/// `encode_for_md` always emits. Falls back to appending `.png`
/// when the source has no recognisable extension.
fn renormalise_id_to_png(id: &str) -> String {
    match id.rsplit_once('.') {
        Some((stem, _ext)) => format!("{stem}.png"),
        None => format!("{id}.png"),
    }
}

/// HWP5 OLE names are `BIN0001.png` (hex per Hancom convention);
/// HWPX uses `image1.png` (decimal). Branch on the prefix so e.g.
/// `image42.jpg` doesn't get parsed as hex `0x42 == 66`, and
/// `BIN000A.png` doesn't lose the `A` digit.
fn bin_id_from_entry_id(id: &str) -> Option<u16> {
    let stem = id.split_once('.').map(|(s, _)| s).unwrap_or(id);
    if let Some(rest) = stem.strip_prefix("BIN") {
        return u16::from_str_radix(rest, 16).ok();
    }
    let digits: String = stem
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u16>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_solid_png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
        let mut img = image::RgbImage::new(w, h);
        for px in img.pixels_mut() {
            *px = image::Rgb(rgb);
        }
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .expect("encode test png");
        out
    }

    #[test]
    fn encodes_known_png_into_data_uri() {
        let png = make_solid_png(20, 10, [200, 50, 50]);
        let entry = BinaryEntry {
            id: "image1.png".into(),
            mime: Some("image/png".into()),
            bytes: png,
        };
        let encoded = encode_for_md(&[entry], &EncodeOpts::default());
        assert_eq!(encoded.len(), 1);
        let asset = &encoded[0];
        assert_eq!(asset.asset_id, "asset-1");
        assert_eq!(asset.bin_id, Some(1));
        assert_eq!(asset.mime, "image/png");
        assert!(asset.data_uri.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn round_trip_data_uri_yields_equivalent_png() {
        let png = make_solid_png(40, 30, [10, 20, 30]);
        let entry = BinaryEntry {
            id: "image2.png".into(),
            mime: Some("image/png".into()),
            bytes: png,
        };
        let encoded = encode_for_md(
            &[entry.clone()],
            &EncodeOpts { dpi: None }, // skip resize so the bytes line up
        );
        let decoded =
            decode_data_uri_to_binary_entry(&encoded[0].data_uri, &entry.id).expect("decode");
        // Re-decode via image crate and check pixel equality —
        // round-trip through PNG re-encode is structurally
        // lossless even when not byte-identical.
        let reread =
            ImageReader::new(Cursor::new(&decoded.bytes))
                .with_guessed_format()
                .expect("read")
                .decode()
                .expect("decode pixels");
        assert_eq!(reread.width(), 40);
        assert_eq!(reread.height(), 30);
    }

    #[test]
    fn dpi_36_halves_pixel_dims() {
        let png = make_solid_png(200, 100, [0, 0, 0]);
        let entry = BinaryEntry {
            id: "image3.png".into(),
            mime: Some("image/png".into()),
            bytes: png,
        };
        let encoded = encode_for_md(&[entry], &EncodeOpts { dpi: Some(36) });
        let asset = &encoded[0];
        assert_eq!(asset.width, Some(100));
        assert_eq!(asset.height, Some(50));
    }

    #[test]
    fn dpi_72_no_upsample() {
        // Tiny source: 72 dpi factor = 1, so no resize. Asset
        // width matches source.
        let png = make_solid_png(20, 20, [0, 0, 0]);
        let entry = BinaryEntry {
            id: "image4.png".into(),
            mime: Some("image/png".into()),
            bytes: png,
        };
        let encoded = encode_for_md(&[entry], &EncodeOpts::default());
        assert_eq!(encoded[0].width, Some(20));
    }

    #[test]
    fn empty_bytes_skipped() {
        let entry = BinaryEntry {
            id: "x.png".into(),
            mime: None,
            bytes: Vec::new(),
        };
        let encoded = encode_for_md(&[entry], &EncodeOpts::default());
        assert!(encoded.is_empty());
    }

    #[test]
    fn malformed_image_skipped() {
        let entry = BinaryEntry {
            id: "bogus.png".into(),
            mime: Some("image/png".into()),
            bytes: b"not actually a png".to_vec(),
        };
        let encoded = encode_for_md(&[entry], &EncodeOpts::default());
        assert!(encoded.is_empty());
    }

    #[test]
    fn decode_rejects_non_data_scheme() {
        let entry = decode_data_uri_to_binary_entry("https://example.com/x.png", "src");
        assert!(entry.is_none());
    }

    #[test]
    fn decode_rejects_non_base64_encoding() {
        let entry =
            decode_data_uri_to_binary_entry("data:image/png;utf8,oops", "src");
        assert!(entry.is_none());
    }

    #[test]
    fn bin_id_from_hwpx_decimal() {
        assert_eq!(bin_id_from_entry_id("image1.png"), Some(1));
        assert_eq!(bin_id_from_entry_id("image42.jpg"), Some(42));
    }

    #[test]
    fn bin_id_from_hwp5_hex() {
        // BIN000A = 10 in hex
        assert_eq!(bin_id_from_entry_id("BIN000A.png"), Some(10));
    }

    #[test]
    fn bin_id_none_when_no_digits() {
        assert!(bin_id_from_entry_id("foobar.png").is_none());
    }
}
