//! Import paths. Reads a foreign source format and produces a
//! neutral [`IrDocument`](hwp_transpiler_core::ir::IrDocument).
//!
//! Markdown is the first source — see [`markdown::from_markdown`].
//! Once the IR is built it can flow into either HWP / HWPX writer
//! to complete the bidirectional round-trip the project's identity
//! depends on.

pub mod cell_sizes;
pub mod markdown;
pub mod markdown_llm;
