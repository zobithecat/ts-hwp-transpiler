//! Neutral in-memory document model. Every Reader produces this; every Writer
//! consumes this. Round-trip fidelity requires holding verbatim bytes for
//! records the IR hasn't been taught about yet — see
//! [`UnknownRecord`](doc_info::UnknownRecord) and [`IrDocument::unknown_streams`].

pub mod body;
pub mod doc_info;

pub use body::{
    CharShapeRun, Control, ControlKind, EquationControl, LineSegment, Paragraph, ParagraphHeader,
    PictureControl, Section, SectionProperties, TableCell, TableControl, ValidZone,
};
pub use doc_info::{
    Border, BorderFill, CharShape, DocInfo, DocProperties, Fill, FontFace, FontFaces,
    FontTypeInfo, ParaShape, Record, Style, SubstituteFont, UnknownRecord,
};

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrDocument {
    pub header: FileHeader,
    pub doc_info: DocInfo,
    pub sections: Vec<Section>,
    pub bin_data: Vec<BinaryEntry>,
    pub preview: Option<Preview>,
    pub scripts: Option<Scripts>,
    pub doc_options: Option<Vec<u8>>,
    pub distribute_doc_data: Option<Vec<u8>>,
    /// Streams whose decoder is not yet implemented. Held verbatim so
    /// `HwpWriter` round-trips them without data loss.
    pub unknown_streams: BTreeMap<String, Vec<u8>>,
}

impl Default for IrDocument {
    fn default() -> Self {
        Self {
            header: FileHeader::default(),
            doc_info: DocInfo::default(),
            sections: Vec::new(),
            bin_data: Vec::new(),
            preview: None,
            scripts: None,
            doc_options: None,
            distribute_doc_data: None,
            unknown_streams: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileHeader {
    pub signature: [u8; 32],
    pub version: u32,
    pub flags: u32,
    /// Bytes 40..256 of the FileHeader stream. HWP5 spec defines sub-fields
    /// (license / encrypt / distribute info + reserved) here; we keep them
    /// verbatim and only crack them open when semantically needed.
    pub reserved: Vec<u8>,
}

impl FileHeader {
    pub const COMPRESSED_BIT: u32 = 0x01;
    pub const ENCRYPTED_BIT: u32 = 0x02;
    pub const DISTRIBUTE_BIT: u32 = 0x04;

    pub fn compressed(&self) -> bool { self.flags & Self::COMPRESSED_BIT != 0 }
    pub fn encrypted(&self) -> bool { self.flags & Self::ENCRYPTED_BIT != 0 }
    pub fn distribute_doc(&self) -> bool { self.flags & Self::DISTRIBUTE_BIT != 0 }
}

impl Default for FileHeader {
    fn default() -> Self {
        let mut sig = [0u8; 32];
        sig[..17].copy_from_slice(b"HWP Document File");
        Self {
            signature: sig,
            version: 0x0005_0501,
            flags: Self::COMPRESSED_BIT,
            reserved: vec![0; 216],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryEntry {
    pub id: String,
    pub mime: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Preview {
    pub text: Option<String>,
    pub image_png: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Scripts {
    pub header: Option<String>,
    pub source: Option<String>,
    pub pre_script: Option<String>,
}

#[derive(thiserror::Error, Debug)]
pub enum IrError {
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("encrypted document (not supported)")]
    Encrypted,
    #[error("io: {0}")]
    Io(String),
}

impl From<std::io::Error> for IrError {
    fn from(e: std::io::Error) -> Self {
        IrError::Io(e.to_string())
    }
}

pub trait Reader {
    fn read(&mut self, input: &[u8]) -> Result<IrDocument, IrError>;
}

pub trait Writer {
    fn write(&mut self, doc: &IrDocument) -> Result<Vec<u8>, IrError>;
}
