//! HWPX (OWPML) codec — ZIP container of XMLs + BinData.
//!
//! Public surface:
//!   * [`HwpxReader`] — bytes → `IrDocument`; parses `Contents/
//!     section*.xml` into typed paragraphs/tables, other parts pass
//!     through `unknown_streams` verbatim.
//!   * [`HwpxWriter`] — still a stub; HWPX write needs a verbatim
//!     container strategy separate from the typed reader.

pub mod reader;
pub mod section_xml;
pub mod writer;
pub mod zip_reader;

pub use reader::HwpxReader;
pub use writer::HwpxWriter;
