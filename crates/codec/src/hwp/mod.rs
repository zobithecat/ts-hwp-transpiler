//! HWP5 binary codec. OLE Compound Document container via `cfb` crate.
//! Stream ordering and record encoding follow `neolord0/hwplib`.

pub mod blank;
pub mod ole;
pub mod reader;
pub mod streams;
pub mod writer;

pub use blank::blank_document;
pub use reader::HwpReader;
pub use writer::HwpWriter;

/// Canonical stream order, per HWP5 spec / hwplib output.
pub const STREAM_ORDER: &[&str] = &[
    "FileHeader",
    "DocInfo",
    "BinData",
    "PrvText",
    "PrvImage",
    "DocOptions",
    "Scripts",
    "DistributeDocData",
];
