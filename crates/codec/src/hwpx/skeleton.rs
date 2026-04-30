//! Minimum-viable HWPX skeleton parts.
//!
//! Used by from-scratch construction paths (e.g. `md-to-hwpx`) where
//! the IR is built from a non-HWPX source and `unknown_streams` is
//! empty. Without these, the writer's output has only `mimetype` +
//! `Contents/section{N}.xml`, which our reader can re-read but most
//! HWPX viewers (Hancom Office / `@rhwp/editor` / hwplib) reject for
//! lacking the OCF rootfile manifest.
//!
//! `bundle_default_skeleton` injects each part only when not already
//! present, so this is safe to call on a doc that already came from
//! a real HWPX read.
//!
//! `Contents/header.xml` uses Start/End container form for
//! `<hh:paraProperties>` / `<hh:charProperties>` (rather than
//! self-closing with `itemCnt="0"`) so the surgical header rewriter's
//! container-end splice path can insert any IR-side shapes that
//! weren't yet present in the bundled skeleton.

use hwp_transpiler_core::ir::{FontFace, FontFaces, IrDocument};

/// Standard OCF container manifest pointing at the HWPX content
/// package.
pub const CONTAINER_XML: &[u8] = CONTAINER_XML_STR.as_bytes();
const CONTAINER_XML_STR: &str = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes" ?><ocf:container xmlns:ocf="urn:oasis:names:tc:opendocument:xmlns:container"><ocf:rootfiles><ocf:rootfile full-path="Contents/content.hpf" media-type="application/hwpml-package+xml"/></ocf:rootfiles></ocf:container>"##;

/// HWPX content-package manifest. Lists `header.xml` and a single
/// `section0.xml`. `BinData/*` items aren't enumerated — viewers
/// resolve those by explicit `binaryItemIDRef` from inside section
/// XML, not via the spine.
pub const CONTENT_HPF: &[u8] = CONTENT_HPF_STR.as_bytes();
const CONTENT_HPF_STR: &str = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes" ?><opf:package xmlns:opf="http://www.idpf.org/2007/opf/" version="" unique-identifier="" id=""><opf:metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><opf:meta name="creator" content="text">md-to-hwpx</opf:meta></opf:metadata><opf:manifest><opf:item id="header" href="Contents/header.xml" media-type="application/xml"/><opf:item id="section0" href="Contents/section0.xml" media-type="application/xml"/></opf:manifest><opf:spine><opf:itemref idref="section0" linear="yes"/></opf:spine></opf:package>"##;

/// Header skeleton: namespaced root with empty containers ready to
/// receive shapes through the surgical rewriter. `<hh:beginNum>`
/// carries the customary "starts at 1" counters; the empty
/// containers preserve the order viewers expect (refList →
/// fontfaces / borderFills / charProperties / paraProperties).
pub const HEADER_XML: &[u8] = HEADER_XML_STR.as_bytes();
const HEADER_XML_STR: &str = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes" ?><hh:head xmlns:ha="http://www.hancom.co.kr/hwpml/2011/app" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core" xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" version="1.4" secCnt="1"><hh:beginNum page="1" footnote="1" endnote="1" pic="1" tbl="1" equation="1"/><hh:refList><hh:fontfaces><hh:fontface lang="HANGUL" fontCnt="1"><hh:font id="0" face="함초롬바탕" type="TTF" isEmbedded="0"/></hh:fontface><hh:fontface lang="LATIN" fontCnt="1"><hh:font id="0" face="함초롬바탕" type="TTF" isEmbedded="0"/></hh:fontface><hh:fontface lang="HANJA" fontCnt="1"><hh:font id="0" face="함초롬바탕" type="TTF" isEmbedded="0"/></hh:fontface><hh:fontface lang="JAPANESE" fontCnt="1"><hh:font id="0" face="함초롬바탕" type="TTF" isEmbedded="0"/></hh:fontface><hh:fontface lang="OTHER" fontCnt="1"><hh:font id="0" face="함초롬바탕" type="TTF" isEmbedded="0"/></hh:fontface><hh:fontface lang="SYMBOL" fontCnt="1"><hh:font id="0" face="함초롬바탕" type="TTF" isEmbedded="0"/></hh:fontface><hh:fontface lang="USER" fontCnt="1"><hh:font id="0" face="함초롬바탕" type="TTF" isEmbedded="0"/></hh:fontface></hh:fontfaces><hh:borderFills><hh:borderFill id="0" threeD="0" shadow="0" centerLine="NONE" breakCellSeparateLine="0"><hh:slash type="NONE" Crooked="0" isCounter="0"/><hh:backSlash type="NONE" Crooked="0" isCounter="0"/><hh:leftBorder type="NONE" width="0.1 mm" color="#000000"/><hh:rightBorder type="NONE" width="0.1 mm" color="#000000"/><hh:topBorder type="NONE" width="0.1 mm" color="#000000"/><hh:bottomBorder type="NONE" width="0.1 mm" color="#000000"/><hh:diagonal type="SOLID" width="0.1 mm" color="#000000"/></hh:borderFill><hh:borderFill id="1" threeD="0" shadow="0" centerLine="NONE" breakCellSeparateLine="0"><hh:slash type="NONE" Crooked="0" isCounter="0"/><hh:backSlash type="NONE" Crooked="0" isCounter="0"/><hh:leftBorder type="SOLID" width="0.12 mm" color="#000000"/><hh:rightBorder type="SOLID" width="0.12 mm" color="#000000"/><hh:topBorder type="SOLID" width="0.12 mm" color="#000000"/><hh:bottomBorder type="SOLID" width="0.12 mm" color="#000000"/><hh:diagonal type="SOLID" width="0.1 mm" color="#000000"/></hh:borderFill></hh:borderFills><hh:charProperties></hh:charProperties><hh:paraProperties></hh:paraProperties><hh:styles></hh:styles></hh:refList></hh:head>"##;

/// HWPX version stamp. Some viewers (notably mac HWP 2014) refuse
/// to open the package when this is missing — the spec calls it
/// optional but a bare-minimum stub keeps parity with Hancom-
/// authored archives.
pub const VERSION_XML: &[u8] = VERSION_XML_STR.as_bytes();
const VERSION_XML_STR: &str = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes" ?><hv:HCFVersion xmlns:hv="http://www.hancom.co.kr/hwpml/2011/version" tagetApplication="WORDPROC" major="5" minor="1" micro="0" buildNumber="0"/>"##;

/// Document-level settings (zoom, view mode, …). Empty stub keeps
/// viewers from complaining about a missing settings.xml; mutations
/// can replace the entry verbatim later.
pub const SETTINGS_XML: &[u8] = SETTINGS_XML_STR.as_bytes();
const SETTINGS_XML_STR: &str = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes" ?><ha:HWPApplicationSetting xmlns:ha="http://www.hancom.co.kr/hwpml/2011/app"/>"##;

/// Inject the default skeleton parts into `doc.unknown_streams` for
/// any path that hasn't already been provided. Idempotent: pre-
/// existing entries (e.g. from a real HWPX read) are preserved.
///
/// When `doc.doc_info.font_faces` is non-empty (HWP5-sourced docs
/// carry 17+ fonts per script), the bundled `Contents/header.xml`
/// gets its empty `<hh:fontfaces>` block substituted with one
/// derived from the IR — otherwise every `<hh:fontRef hangul="N">`
/// in section bodies references a slot that doesn't exist (the
/// skeleton ships a single `id="0"` placeholder), the viewer falls
/// back to its default font, and the rendered text uses the wrong
/// typeface for every glyph.
pub fn bundle_default_skeleton(doc: &mut IrDocument) {
    let entries: &[(&str, &[u8])] = &[
        ("META-INF/container.xml", CONTAINER_XML),
        ("Contents/content.hpf", CONTENT_HPF),
        ("Contents/header.xml", HEADER_XML),
        ("settings.xml", SETTINGS_XML),
        ("version.xml", VERSION_XML),
    ];
    for (path, bytes) in entries {
        doc.unknown_streams
            .entry((*path).to_string())
            .or_insert_with(|| bytes.to_vec());
    }

    // Substitute the bundled header's fontfaces block with one built
    // from the IR's actual fonts. Only runs when (a) the header is
    // exactly the bundled stub (so we don't perturb real HWPX
    // headers) and (b) the IR carries any font face entries.
    if let Some(header) = doc.unknown_streams.get_mut("Contents/header.xml") {
        if header.as_slice() == HEADER_XML && fontfaces_has_content(&doc.doc_info.font_faces) {
            let new = substitute_fontfaces_in_header(header, &doc.doc_info.font_faces);
            *header = new;
        }
    }
}

fn fontfaces_has_content(f: &FontFaces) -> bool {
    !f.hangul.is_empty()
        || !f.latin.is_empty()
        || !f.hanja.is_empty()
        || !f.japanese.is_empty()
        || !f.other.is_empty()
        || !f.symbol.is_empty()
        || !f.user.is_empty()
}

/// Replace the `<hh:fontfaces>...</hh:fontfaces>` span in `header`
/// with one populated from the IR's per-script font tables. Each
/// non-empty script lang gets its own `<hh:fontface lang="…"
/// fontCnt="N">` container with one `<hh:font id="i" face="…">`
/// child per IR entry. Empty scripts emit a single placeholder so
/// `<hh:fontRef ...="0">` references stay resolvable.
fn substitute_fontfaces_in_header(header: &[u8], fonts: &FontFaces) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(header) else {
        return header.to_vec();
    };
    let Some(open_pos) = text.find("<hh:fontfaces>") else {
        return header.to_vec();
    };
    let close_marker = "</hh:fontfaces>";
    let Some(close_pos) = text[open_pos..].find(close_marker) else {
        return header.to_vec();
    };
    let close_end = open_pos + close_pos + close_marker.len();

    let mut block = String::new();
    block.push_str("<hh:fontfaces>");
    for (lang, faces) in [
        ("HANGUL", &fonts.hangul),
        ("LATIN", &fonts.latin),
        ("HANJA", &fonts.hanja),
        ("JAPANESE", &fonts.japanese),
        ("OTHER", &fonts.other),
        ("SYMBOL", &fonts.symbol),
        ("USER", &fonts.user),
    ] {
        block.push_str(&render_fontface_lang(lang, faces));
    }
    block.push_str("</hh:fontfaces>");

    let mut out = Vec::with_capacity(text.len() + block.len());
    out.extend_from_slice(text[..open_pos].as_bytes());
    out.extend_from_slice(block.as_bytes());
    out.extend_from_slice(text[close_end..].as_bytes());
    out
}

fn render_fontface_lang(lang: &str, faces: &[FontFace]) -> String {
    if faces.is_empty() {
        // Always emit at least one placeholder per script so
        // `<hh:fontRef ...="0">` resolves regardless of which lang
        // a paragraph references.
        return format!(
            r#"<hh:fontface lang="{lang}" fontCnt="1"><hh:font id="0" face="함초롬바탕" type="TTF" isEmbedded="0"/></hh:fontface>"#
        );
    }
    let mut s = format!(r#"<hh:fontface lang="{lang}" fontCnt="{}">"#, faces.len());
    for (i, ff) in faces.iter().enumerate() {
        let face = xml_escape(&ff.name);
        s.push_str(&format!(
            r#"<hh:font id="{i}" face="{face}" type="TTF" isEmbedded="0"/>"#,
        ));
    }
    s.push_str("</hh:fontface>");
    s
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skeleton_added_when_streams_empty() {
        let mut doc = IrDocument::default();
        bundle_default_skeleton(&mut doc);
        assert!(doc.unknown_streams.contains_key("META-INF/container.xml"));
        assert!(doc.unknown_streams.contains_key("Contents/content.hpf"));
        assert!(doc.unknown_streams.contains_key("Contents/header.xml"));
    }

    #[test]
    fn existing_entries_are_not_overwritten() {
        let mut doc = IrDocument::default();
        doc.unknown_streams
            .insert("Contents/header.xml".into(), b"<custom/>".to_vec());
        bundle_default_skeleton(&mut doc);
        assert_eq!(
            doc.unknown_streams["Contents/header.xml"],
            b"<custom/>"
        );
    }
}
