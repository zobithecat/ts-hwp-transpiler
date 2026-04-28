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

use hwp_transpiler_core::ir::IrDocument;

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
const HEADER_XML_STR: &str = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes" ?><hh:head xmlns:ha="http://www.hancom.co.kr/hwpml/2011/app" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core" xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" version="1.4" secCnt="1"><hh:beginNum page="1" footnote="1" endnote="1" pic="1" tbl="1" equation="1"/><hh:refList><hh:fontfaces><hh:fontface lang="HANGUL" fontCnt="1"><hh:font id="0" face="함초롬바탕" type="TTF" isEmbedded="0"/></hh:fontface><hh:fontface lang="LATIN" fontCnt="1"><hh:font id="0" face="함초롬바탕" type="TTF" isEmbedded="0"/></hh:fontface><hh:fontface lang="HANJA" fontCnt="1"><hh:font id="0" face="함초롬바탕" type="TTF" isEmbedded="0"/></hh:fontface><hh:fontface lang="JAPANESE" fontCnt="1"><hh:font id="0" face="함초롬바탕" type="TTF" isEmbedded="0"/></hh:fontface><hh:fontface lang="OTHER" fontCnt="1"><hh:font id="0" face="함초롬바탕" type="TTF" isEmbedded="0"/></hh:fontface><hh:fontface lang="SYMBOL" fontCnt="1"><hh:font id="0" face="함초롬바탕" type="TTF" isEmbedded="0"/></hh:fontface><hh:fontface lang="USER" fontCnt="1"><hh:font id="0" face="함초롬바탕" type="TTF" isEmbedded="0"/></hh:fontface></hh:fontfaces><hh:borderFills><hh:borderFill id="0"></hh:borderFill><hh:borderFill id="1" threeD="0" shadow="0" centerLine="NONE" breakCellSeparateLine="0"><hh:slash type="NONE" Crooked="0" isCounter="0"/><hh:backSlash type="NONE" Crooked="0" isCounter="0"/><hh:leftBorder type="SOLID" width="0.12 mm" color="#000000"/><hh:rightBorder type="SOLID" width="0.12 mm" color="#000000"/><hh:topBorder type="SOLID" width="0.12 mm" color="#000000"/><hh:bottomBorder type="SOLID" width="0.12 mm" color="#000000"/><hh:diagonal type="NONE" Crooked="0" isCounter="0"/></hh:borderFill></hh:borderFills><hh:charProperties></hh:charProperties><hh:paraProperties></hh:paraProperties><hh:styles></hh:styles></hh:refList></hh:head>"##;

/// Inject the default skeleton parts into `doc.unknown_streams` for
/// any path that hasn't already been provided. Idempotent: pre-
/// existing entries (e.g. from a real HWPX read) are preserved.
pub fn bundle_default_skeleton(doc: &mut IrDocument) {
    let entries: &[(&str, &[u8])] = &[
        ("META-INF/container.xml", CONTAINER_XML),
        ("Contents/content.hpf", CONTENT_HPF),
        ("Contents/header.xml", HEADER_XML),
    ];
    for (path, bytes) in entries {
        doc.unknown_streams
            .entry((*path).to_string())
            .or_insert_with(|| bytes.to_vec());
    }
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
