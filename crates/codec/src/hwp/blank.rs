//! Bootstrap: produce a valid empty `IrDocument` without a filesystem
//! seed, so callers building HWP files from nothing (MD → HWP path,
//! unit tests, CLI scaffolds) can start from a known-good shape.
//!
//! **Current implementation is template-driven**: the vendored hwplib
//! `blank.hwp` bytes are embedded via [`include_bytes!`] and parsed on
//! demand. This is honest about what we can and can't generate today —
//! many DocInfo records (ID_MAPPINGS, FACE_NAME, NUMBERING, track-change
//! blocks, ...) and BodyText control records (section-def `dces`,
//! column-def `dloc`) don't have typed encoders yet, so constructing
//! them from code would require hand-written byte payloads that would
//! duplicate hwplib's authoritative output anyway.
//!
//! Future work: replace the parse of the embedded template with direct
//! IR construction as typed encoders land for each missing record
//! type. The public signature doesn't change — callers see the same
//! [`IrDocument`] either way.

use hwp_transpiler_core::ir::{IrDocument, IrError, Reader};

use super::reader::HwpReader;

/// Vendored hwplib `BlankFileMaker` output. Committed at
/// `crates/codec/tests/fixtures/blank.hwp` and embedded here so the
/// codec library can hand out a valid empty document without a
/// filesystem read. ~22 KB — measurable but negligible in the context
/// of a codec that embeds image and font tables.
const HWPLIB_BLANK: &[u8] = include_bytes!("../../tests/fixtures/blank.hwp");

/// Return a valid empty `IrDocument` equivalent to hwplib's
/// `BlankFileMaker.make()` output. Round-tripping this through
/// [`HwpWriter`](super::writer::HwpWriter) yields a `/FileHeader` +
/// `/DocInfo` + `/BodyText/Section0` structure that 한컴 Office can
/// open, and provides a starting seed for callers that want to add
/// paragraphs on top.
pub fn blank_document() -> Result<IrDocument, IrError> {
    HwpReader.read(HWPLIB_BLANK)
}
