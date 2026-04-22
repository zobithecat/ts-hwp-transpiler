//! Export paths. Consumes `IrDocument`, produces a target format.
//! Markdown is the first target — see [`markdown::to_markdown`].
//!
//! `assets::dump_assets` is the sidecar binary-entry dumper used by the
//! CLI to make `![](…)` links in the emitted Markdown resolve on disk.

pub mod assets;
pub mod markdown;
