//! Markdown Hinting Layer. HWP CharShape / ParagraphShape carry ~30 fields
//! each; the MD output preserves the visually-meaningful subset as an HTML
//! comment so renderers that understand it can reconstruct the original
//! appearance, while plain MD renderers simply ignore the comment.
//!
//! Emission format: `<!-- hint: {"kind":"char", ...} -->`

pub mod char_hint;
pub mod para_hint;

pub use char_hint::{CharHint, ScriptKind};
pub use para_hint::{Align, ParaHint};

use serde_json::Value;

pub trait Hintable {
    fn to_hint(&self) -> Value;
}

pub fn emit_hint_comment<H: Hintable>(h: &H) -> String {
    let payload = serde_json::to_string(&h.to_hint()).unwrap_or_else(|_| "{}".to_string());
    format!("<!-- hint: {payload} -->")
}
