//! hwp-transpiler core — parser-agnostic IR, visual + semantic extraction,
//! formula tokenizer, Markdown hinting layer.
//!
//! Symmetric pipeline: `Reader → IrDocument → Writer` in both directions.
//! HWP/HWPX/Markdown readers and writers live in sibling crates; `core`
//! defines only the neutral types and trait pairs.

pub mod formula;
pub mod hint;
pub mod ir;
pub mod semantics;
