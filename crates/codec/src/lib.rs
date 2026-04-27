//! HWP / HWPX codec (reader + writer). Consumes and produces
//! [`hwp_transpiler_core::ir::IrDocument`].
//!
//! **Strangler-fig strategy**: the `HwpReader` dumps every OLE stream into
//! `IrDocument::unknown_streams` verbatim, and `HwpWriter` writes them
//! back out verbatim. Round-trip byte equality (at stream-content level)
//! holds from day one. As typed encoders land for DocInfo, BodyText, etc.,
//! each stream migrates out of `unknown_streams` into the typed IR and the
//! round-trip invariant must stay green.

pub mod export;
pub mod hwp;
pub mod hwpx;
pub mod import;
