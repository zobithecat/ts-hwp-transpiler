//! `/DocInfo` — compressed record tree carrying document-wide metadata
//! (shapes, styles, fonts, id mappings, …).
//!
//! Known record tags (from HWP5 spec / hwplib `HWPTag.java`):
//!
//! | tag    | meaning              |
//! |--------|----------------------|
//! | 0x0010 | DocumentProperties   |
//! | 0x0011 | IdMappings           |
//! | 0x0012 | BinData              |
//! | 0x0013 | FaceName             |
//! | 0x0014 | BorderFill           |
//! | 0x0015 | CharShape            |
//! | 0x0016 | TabDef               |
//! | 0x0017 | Numbering            |
//! | 0x0018 | Bullet               |
//! | 0x0019 | ParaShape            |
//! | 0x001A | Style                |
//! | 0x001B | DocData              |
//! | 0x001C | DistributeDocData    |
//! | 0x001E | CompatibleDocument   |
//! | 0x001F | LayoutCompatibility  |
//! | 0x0020 | TrackChange (info)   |
//! | 0x005C | MemoShape            |
//! | 0x005E | ForbiddenChar        |
//! | 0x0060 | TrackChange (body)   |
//! | 0x0061 | TrackChangeAuthor    |
//!
//! Compression: when FileHeader.compressed = 1, the stream body is raw
//! DEFLATE (RFC 1951) — *no* zlib header.

use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use hwp_transpiler_core::ir::{IrError, Record};
use std::io::{Read, Write};

use super::record;

pub const STREAM_NAME: &str = "/DocInfo";

pub mod tag {
    pub const DOCUMENT_PROPERTIES: u16 = 0x0010;
    pub const ID_MAPPINGS: u16 = 0x0011;
    pub const BIN_DATA: u16 = 0x0012;
    pub const FACE_NAME: u16 = 0x0013;
    pub const BORDER_FILL: u16 = 0x0014;
    pub const CHAR_SHAPE: u16 = 0x0015;
    pub const TAB_DEF: u16 = 0x0016;
    pub const NUMBERING: u16 = 0x0017;
    pub const BULLET: u16 = 0x0018;
    pub const PARA_SHAPE: u16 = 0x0019;
    pub const STYLE: u16 = 0x001A;
    pub const DOC_DATA: u16 = 0x001B;
    pub const DISTRIBUTE_DOC_DATA: u16 = 0x001C;
    pub const COMPATIBLE_DOCUMENT: u16 = 0x001E;
    pub const LAYOUT_COMPATIBILITY: u16 = 0x001F;
    /// Track-changes metadata block. Layout undocumented in the public
    /// HWP5 spec; preserved verbatim via `raw_records` until typed.
    pub const TRACK_CHANGE_INFO: u16 = 0x0020;
    pub const MEMO_SHAPE: u16 = 0x005C;
    /// Korean line-break forbidden-character set. Like the others below,
    /// preserved verbatim until a fixture exercises it.
    pub const FORBIDDEN_CHAR: u16 = 0x005E;
    /// Track-changes body record (paired with TRACK_CHANGE_AUTHOR).
    pub const TRACK_CHANGE: u16 = 0x0060;
    pub const TRACK_CHANGE_AUTHOR: u16 = 0x0061;
}

pub fn decompress(compressed: &[u8]) -> Result<Vec<u8>, IrError> {
    let mut out = Vec::new();
    DeflateDecoder::new(compressed)
        .read_to_end(&mut out)
        .map_err(|e| IrError::Invalid(format!("DocInfo decompress: {e}")))?;
    Ok(out)
}

pub fn compress(raw: &[u8]) -> Result<Vec<u8>, IrError> {
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
    enc.write_all(raw)
        .map_err(|e| IrError::Io(e.to_string()))?;
    enc.finish().map_err(|e| IrError::Io(e.to_string()))
}

pub fn parse(compressed: &[u8]) -> Result<Vec<Record>, IrError> {
    let raw = decompress(compressed)?;
    record::parse_all(&raw)
}

pub fn emit(records: &[Record]) -> Result<Vec<u8>, IrError> {
    let raw = record::emit_all(records);
    compress(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_survive_compress_decompress() {
        let rs = vec![
            Record { tag: tag::DOCUMENT_PROPERTIES, level: 0, data: vec![0; 30] },
            Record {
                tag: tag::FACE_NAME,
                level: 0,
                data: b"NanumGothic".to_vec(),
            },
            Record { tag: tag::CHAR_SHAPE, level: 0, data: vec![0xAA; 72] },
        ];
        let compressed = emit(&rs).unwrap();
        let parsed = parse(&compressed).unwrap();
        assert_eq!(parsed, rs);
    }

    #[test]
    fn raw_bytes_survive_compress_decompress() {
        let raw = b"arbitrary binary \x00\x01\x02\xFF payload";
        let back = decompress(&compress(raw).unwrap()).unwrap();
        assert_eq!(back, raw);
    }
}
