//! HWPX container writer.
//!
//! Strategy — same "verbatim cache for parts not yet typed" shape as
//! the HWP5 writer: sections round-trip through a fresh XML emit
//! (since the reader decoded them into typed IR); the original
//! `Contents/header.xml` flows through a *surgical rewriter* that
//! overlays only the attributes the IR exposes (paraPr align, charPr
//! parent attrs, font face names, borderFill solid colours, strike /
//! underline shape attrs); every other archive part rides along
//! verbatim from `doc.unknown_streams`. This lets a DocInfo-side
//! mutation flow into the written file without re-emitting from
//! scratch (which would lose the unparsed sections — styles,
//! numberings, lineSpacing, kerning flags, Panose typeInfo).
//!
//! Missing compared to a full writer:
//!   * fontface add / remove — entries are partitioned by `lang`
//!     slot (HANGUL / LATIN / …), each with its own count. Per-slot
//!     accounting needs separate work. paraPr / charPr add+remove is
//!     supported via the rewriter's container-end splice path.
//!   * Gradation / image fill mutation — the IR's `Fill` doesn't
//!     reflect these as typed fields, so the rewriter can't safely
//!     overlay them. Solid-color fills are supported.
//!   * Pictures / equations in paragraph controls don't emit back
//!     into the XML; they still exist in the IR and would need their
//!     own serialisers to reach the output.
//!   * New sections beyond what the reader saw aren't referenced from
//!     `Contents/content.hpf` (which stays verbatim); viewers may
//!     ignore the extras.

use hwp_transpiler_core::ir::{
    BinaryEntry, ControlKind, IrDocument, IrError, Paragraph, Writer,
};

use super::header_rewriter::rewrite_header_xml;
use super::section_writer::write_section_xml;
use super::zip_writer::HwpxArchiveWriter;

const MIMETYPE: &str = "application/hwp+zip";

#[derive(Default)]
pub struct HwpxWriter;

impl Writer for HwpxWriter {
    fn write(&mut self, doc: &IrDocument) -> Result<Vec<u8>, IrError> {
        let mut zip = HwpxArchiveWriter::new();

        // OCF requires mimetype first, stored uncompressed.
        zip.write_mimetype(MIMETYPE)?;

        // Prefer verbatim cached bytes when the section hasn't been
        // mutated — the typed-IR re-emit path drops elements the
        // parser doesn't yet understand (`<hp:linesegarray>`,
        // `<hp:secPr>`, paragraph layout metadata) and viewers
        // notice. Mutating helpers in `Section` clear the cache so
        // a fresh emit happens whenever the IR shape actually
        // changed.
        for (i, section) in doc.sections.iter().enumerate() {
            let bytes = match &section.stream_bytes {
                // Verbatim only when the cache actually IS HWPX XML —
                // a HWP5 reader populates `stream_bytes` with the
                // binary `BodyText/Section{N}` blob, and dumping that
                // into `Contents/section{N}.xml` produces an archive
                // every HWPX consumer rejects mid-parse with
                // "tag not closed". Sniff the leading bytes; fall
                // back to the typed XML emitter for non-XML payloads.
                Some(cached) if looks_like_xml(cached) => cached.clone(),
                _ => write_section_xml(section, &doc.bin_data)?,
            };
            zip.add_part(&format!("Contents/section{i}.xml"), &bytes)?;
        }

        // Re-emit embedded binaries under `BinData/<id>` — the reader
        // promotes those out of `unknown_streams` into `doc.bin_data`
        // so the HTML preview can resolve `binaryItemIDRef`; the
        // writer has to put them back.
        //
        // Order: zip-emit `bin_data` in the order each entry is FIRST
        // referenced by a picture in the document (orphans last). Some
        // viewers (rhwp, mac HWP 2014) bind picture → BinData by zip-
        // entry sequence rather than via the manifest's
        // `binaryItemIDRef` lookup. When the IR carried `[image2,
        // image1]` (zip-encounter order from the original) but the
        // first picture in section0 referenced `image1`, those viewers
        // showed the images swapped. Reordering at write time keeps
        // manifest-based viewers happy (they ignore order) while
        // fixing positional viewers.
        let ordered = bin_data_in_picture_order(doc);
        for entry in ordered {
            if entry.bytes.is_empty() {
                continue;
            }
            zip.add_part(&format!("BinData/{}", entry.id), &entry.bytes)?;
        }

        // Passthrough every other archive part verbatim. Section
        // entries from the original are skipped because we just
        // emitted fresh ones — if an IR-side edit changed the
        // section count the original entries would conflict otherwise.
        // `mimetype` is also skipped because `write_mimetype` already
        // placed it, stored uncompressed. `BinData/*` is skipped
        // because bin_data above already handled it.
        for (name, bytes) in &doc.unknown_streams {
            if name == "mimetype" {
                continue;
            }
            if is_section_xml(name) {
                continue;
            }
            if name.starts_with("BinData/") {
                continue;
            }
            if !is_hwpx_path(name) {
                // HWP5 → HWPX cross-format conversion: the HWP5 reader
                // surfaces OLE compound-file streams (e.g.
                // `/\x05HwpSummaryInformation`, `/PrvImage`,
                // `/Scripts/DefaultJScript`) into `unknown_streams`.
                // Those names aren't valid HWPX paths and embedding
                // them straight into the OCF zip produces an archive
                // HWP 2014 / rhwp refuse to open. Drop anything that
                // doesn't match a recognised HWPX prefix; the cross-
                // format converter doesn't preserve HWP5 metadata
                // through the round-trip yet.
                continue;
            }
            if name == "Contents/header.xml" {
                // Overlay IR-side DocInfo mutations on top of the
                // original header.xml bytes. Falls back to the
                // verbatim original if the rewriter chokes on the
                // input — better to ship an unmutated header than a
                // corrupted one.
                let rewritten = rewrite_header_xml(bytes, doc).unwrap_or_else(|_| bytes.clone());
                zip.add_part(name, &rewritten)?;
                continue;
            }
            if name == "Contents/content.hpf" {
                // Splice an `<opf:item>` per `BinaryEntry` into the
                // package manifest. Hancom viewers resolve picture
                // `binaryItemIDRef` references through this list
                // rather than scanning `BinData/`, so a missing
                // entry shows the picture as broken even when the
                // bytes are present in the archive.
                let rewritten = inject_bin_data_into_manifest(bytes, doc);
                zip.add_part(name, &rewritten)?;
                continue;
            }
            zip.add_part(name, bytes)?;
        }

        zip.finish()
    }
}

/// Sort `bin_data` so the first-referenced picture's bytes land in
/// the zip first, second-referenced second, and so on. Entries that
/// no picture references (or whose `bin_id` doesn't parse) come at
/// the end, in their original Vec order.
fn bin_data_in_picture_order(doc: &IrDocument) -> Vec<&BinaryEntry> {
    let mut order: Vec<u16> = Vec::new();
    let mut seen: std::collections::HashSet<u16> = std::collections::HashSet::new();
    for section in &doc.sections {
        for para in &section.paragraphs {
            collect_picture_bin_ids(para, &mut order, &mut seen);
        }
    }
    let mut taken = vec![false; doc.bin_data.len()];
    let mut out: Vec<&BinaryEntry> = Vec::with_capacity(doc.bin_data.len());
    for bin_id in order {
        for (i, entry) in doc.bin_data.iter().enumerate() {
            if !taken[i] && entry_bin_id(entry) == Some(bin_id) {
                taken[i] = true;
                out.push(entry);
                break;
            }
        }
    }
    for (i, entry) in doc.bin_data.iter().enumerate() {
        if !taken[i] {
            out.push(entry);
        }
    }
    out
}

/// Walk a paragraph's controls (and any nested table cells'
/// paragraphs) collecting picture `bin_id`s in document order.
fn collect_picture_bin_ids(
    para: &Paragraph,
    order: &mut Vec<u16>,
    seen: &mut std::collections::HashSet<u16>,
) {
    for ctrl in &para.controls {
        match &ctrl.kind {
            ControlKind::Picture(pic) => {
                if seen.insert(pic.bin_id) {
                    order.push(pic.bin_id);
                }
            }
            ControlKind::Table(tbl) => {
                for cell in &tbl.cells {
                    for sub in &cell.paragraphs {
                        collect_picture_bin_ids(sub, order, seen);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Parse the numeric `bin_id` out of a `BinaryEntry::id` like
/// `image1.png` → 1 or `BIN0001.png` → 1. Mirrors
/// `asset_pipeline::bin_id_from_entry_id` but kept local to avoid a
/// cross-module dep here.
fn entry_bin_id(entry: &BinaryEntry) -> Option<u16> {
    let stem = entry
        .id
        .split_once('.')
        .map(|(s, _)| s)
        .unwrap_or(&entry.id);
    if let Some(rest) = stem.strip_prefix("BIN") {
        return u16::from_str_radix(rest, 16).ok();
    }
    let digits: String = stem
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse::<u16>().ok()
}

/// Rewrite the package manifest's `BinData/*` section so it matches
/// the actual archive contents and ordering:
///
/// 1. Strip every existing `<opf:item ... href="BinData/..." .../>`
///    (dangling AND valid). Hancom-authored docs sometimes carry
///    duplicate / stale BinData references — e.g. an `image1.jpg`
///    `<opf:item>` survives even after the underlying file was
///    replaced with `image1.png`. When two items share `id="image1"`
///    and the first href doesn't resolve, rhwp / mac HWP 2014 latch
///    onto the broken one and pictures show up swapped, on the wrong
///    page, or not at all.
/// 2. Re-emit one `<opf:item>` per real `doc.bin_data` entry, in
///    picture-reference order (same order the writer feeds entries
///    into the zip), spliced before the first
///    `<opf:item id="section…" />`. Mirrors the Hancom layout
///    (header → images → sections → settings) viewers expect.
///
/// Non-BinData items (header, sections, settings, …) flow through
/// unchanged so we don't perturb anything we don't have to.
fn inject_bin_data_into_manifest(content_hpf: &[u8], doc: &IrDocument) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(content_hpf) else {
        return content_hpf.to_vec();
    };

    // Pass 1: strip every BinData item (dangling AND valid). We
    // re-add the valid ones in a controlled order in pass 2 so the
    // manifest sequence matches picture-reference order, regardless
    // of however Hancom shipped the original.
    let mut pruned = String::with_capacity(text.len());
    let mut cursor = 0usize;
    while let Some(rel_open) = text[cursor..].find("<opf:item") {
        let abs_open = cursor + rel_open;
        // Carry over everything before the tag.
        pruned.push_str(&text[cursor..abs_open]);
        // Find the tag's `/>` close. Manifest items are self-closing.
        let after_open = &text[abs_open..];
        let Some(rel_close) = after_open.find("/>") else {
            // Malformed — bail out and keep the rest verbatim.
            pruned.push_str(after_open);
            cursor = text.len();
            break;
        };
        let abs_close = abs_open + rel_close + 2;
        let tag = &text[abs_open..abs_close];
        let is_bindata = tag.contains(r#"href="BinData/"#);
        if !is_bindata {
            pruned.push_str(tag);
        }
        // is_bindata items are dropped — pass 2 emits a single, clean
        // entry per real bin_data entry in picture-reference order.
        cursor = abs_close;
    }
    pruned.push_str(&text[cursor..]);

    // Pass 2: splice missing entries.
    let close = "</opf:manifest>";
    let Some(close_pos) = pruned.find(close) else {
        return pruned.into_bytes();
    };
    let manifest_body = &pruned[..close_pos];
    let anchor_pos = manifest_body
        .find(r#"id="section"#)
        .and_then(|i| manifest_body[..i].rfind("<opf:item"))
        .unwrap_or(close_pos);
    // Emit one `<opf:item>` per real BinData entry, in
    // picture-reference order so positional viewers map pic[N] →
    // BinData[N] correctly. Pass 1 stripped every existing BinData
    // tag, so we don't have to dedupe here.
    let mut additions = String::new();
    for entry in bin_data_in_picture_order(doc) {
        if entry.bytes.is_empty() {
            continue;
        }
        let stem = entry
            .id
            .split_once('.')
            .map(|(s, _)| s)
            .unwrap_or(&entry.id);
        let mime = mime_for_manifest(&entry.id);
        additions.push_str(&format!(
            r#"<opf:item id="{stem}" href="BinData/{name}" media-type="{mime}" isEmbeded="1"/>"#,
            stem = stem,
            name = entry.id,
            mime = mime,
        ));
    }
    if additions.is_empty() {
        return pruned.into_bytes();
    }
    let mut out = String::with_capacity(pruned.len() + additions.len());
    out.push_str(&pruned[..anchor_pos]);
    out.push_str(&additions);
    out.push_str(&pruned[anchor_pos..]);
    out.into_bytes()
}

/// Hancom-flavoured mime lookup. `.jpg` / `.jpeg` collapse to
/// `image/jpg` (matching Hancom-authored manifests), other
/// extensions use the standard mime.
fn mime_for_manifest(id: &str) -> &'static str {
    let ext = id.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpg",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("tif") | Some("tiff") => "image/tiff",
        _ => "application/octet-stream",
    }
}

fn is_section_xml(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("Contents/section") else {
        return false;
    };
    let Some(idx) = rest.strip_suffix(".xml") else {
        return false;
    };
    !idx.is_empty() && idx.chars().all(|c| c.is_ascii_digit())
}

/// Cheap "is this XML?" check — used to gate the verbatim section
/// passthrough. Skips ASCII whitespace + the optional UTF-8 BOM,
/// then requires `<` so a HWP5 binary `BodyText/Section{N}` cache
/// (which never starts with `<`) takes the fresh-emit path. We
/// don't validate the XML here; a malformed declaration is the
/// reader's problem, not ours.
pub(crate) fn looks_like_xml(bytes: &[u8]) -> bool {
    let mut i = 0;
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        i = 3;
    }
    while let Some(&b) = bytes.get(i) {
        if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
            i += 1;
            continue;
        }
        return b == b'<';
    }
    false
}

/// Path-prefix whitelist for HWPX (OCF) container parts. Anything
/// not on this list is HWP5 OLE leakage when the IR came from the
/// `.hwp` reader and shouldn't be written into a `.hwpx` archive.
fn is_hwpx_path(name: &str) -> bool {
    name == "mimetype"
        || name == "settings.xml"
        || name == "version.xml"
        || name.starts_with("Contents/")
        || name.starts_with("BinData/")
        || name.starts_with("META-INF/")
        || name.starts_with("Preview/")
        || name.starts_with("Scripts/")
        || name.starts_with("Charts/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use hwp_transpiler_core::ir::{Control, Paragraph, PictureControl, Reader, Section};

    fn doc_with_one_paragraph(text: &str) -> IrDocument {
        let mut doc = IrDocument::default();
        let mut section = Section::default();
        section.paragraphs.push(Paragraph {
            text: text.into(),
            ..Paragraph::default()
        });
        doc.sections.push(section);
        doc
    }

    #[test]
    fn emits_valid_zip_with_pk_magic() {
        let doc = doc_with_one_paragraph("hi");
        let bytes = HwpxWriter::default().write(&doc).expect("write");
        assert_eq!(&bytes[0..4], b"PK\x03\x04");
    }

    #[test]
    fn mimetype_is_first_entry() {
        let doc = doc_with_one_paragraph("hi");
        let bytes = HwpxWriter::default().write(&doc).expect("write");
        let off = 30 + "mimetype".len();
        assert_eq!(&bytes[off..off + MIMETYPE.len()], MIMETYPE.as_bytes());
    }

    #[test]
    fn round_trip_through_reader_preserves_paragraph_text() {
        let doc = doc_with_one_paragraph("round trip");
        let bytes = HwpxWriter::default().write(&doc).expect("write");
        // Re-read the emitted archive to confirm our section XML is
        // structurally valid and our reader agrees.
        let mut r = super::super::reader::HwpxReader;
        let read = r.read(&bytes).expect("read");
        assert_eq!(read.sections.len(), 1);
        // The typed emitter prepends a synthetic secPr paragraph at
        // id=0, so the user's content paragraph isn't necessarily
        // first. Scan for it instead of indexing.
        assert!(
            read.sections[0]
                .paragraphs
                .iter()
                .any(|p| p.text == "round trip"),
            "expected text 'round trip' in some paragraph, got: {:?}",
            read.sections[0]
                .paragraphs
                .iter()
                .map(|p| &p.text)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn is_section_xml_matches_expected_paths() {
        assert!(is_section_xml("Contents/section0.xml"));
        assert!(is_section_xml("Contents/section9.xml"));
        assert!(!is_section_xml("Contents/section.xml"));
        assert!(!is_section_xml("Contents/header.xml"));
        assert!(!is_section_xml("BinData/image1.png"));
    }

    fn pic_para(bin_id: u16) -> Paragraph {
        Paragraph {
            text: "\u{FFFC}".into(),
            controls: vec![Control {
                kind: ControlKind::Picture(PictureControl {
                    bin_id,
                    width_hwpu: 100,
                    height_hwpu: 100,
                }),
                caption_text: None,
            }],
            ..Paragraph::default()
        }
    }

    #[test]
    fn bin_data_reordered_to_picture_appearance() {
        // Section references image1 first then image2, but the IR's
        // bin_data was loaded in zip-encounter order (image2 first).
        // The reorder helper must put image1's bytes ahead of
        // image2's so positional viewers (rhwp / mac HWP 2014) bind
        // pic[0]→image1 instead of pic[0]→image2.
        let mut doc = IrDocument::default();
        doc.bin_data.push(BinaryEntry {
            id: "image2.png".into(),
            mime: Some("image/png".into()),
            bytes: vec![2; 8],
        });
        doc.bin_data.push(BinaryEntry {
            id: "image1.png".into(),
            mime: Some("image/png".into()),
            bytes: vec![1; 8],
        });
        let mut section = Section::default();
        section.paragraphs.push(pic_para(1));
        section.paragraphs.push(pic_para(2));
        doc.sections.push(section);

        let ordered = bin_data_in_picture_order(&doc);
        let ids: Vec<&str> = ordered.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["image1.png", "image2.png"]);
    }

    #[test]
    fn bin_data_orphans_appended_in_original_order() {
        // Pictures only reference image1 — image2 has no picture, so
        // it should land at the end of the zip, preserving the
        // original Vec position relative to other orphans.
        let mut doc = IrDocument::default();
        doc.bin_data.push(BinaryEntry {
            id: "image2.png".into(),
            mime: Some("image/png".into()),
            bytes: vec![2; 8],
        });
        doc.bin_data.push(BinaryEntry {
            id: "image1.png".into(),
            mime: Some("image/png".into()),
            bytes: vec![1; 8],
        });
        let mut section = Section::default();
        section.paragraphs.push(pic_para(1));
        doc.sections.push(section);

        let ordered = bin_data_in_picture_order(&doc);
        let ids: Vec<&str> = ordered.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["image1.png", "image2.png"]);
    }
}
