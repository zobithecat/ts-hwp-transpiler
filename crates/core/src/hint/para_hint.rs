//! ParagraphShape → hint JSON.

use super::Hintable;
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Align {
    Justify,
    Left,
    Right,
    Center,
    Distribute,
    DistributeSpace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LineSpacingKind {
    Percent,
    Fixed,
    BetweenLine,
    AtLeast,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ParaHint {
    pub align: Align,
    pub line_spacing_kind: LineSpacingKind,
    /// Raw HWP value. Percent mode: 100 = single-line; fixed/atleast: centipt.
    pub line_spacing: u32,
    pub indent_pt: f32,
    pub padding_left_pt: f32,
    pub padding_right_pt: f32,
    pub margin_top_pt: f32,
    pub margin_bottom_pt: f32,
    pub heading_level: Option<u8>,
    pub keep_with_next: bool,
    pub page_break_before: bool,
}

impl Hintable for ParaHint {
    fn to_hint(&self) -> Value {
        json!({
            "kind": "para",
            "align": align_str(self.align),
            "lineSpacing": {
                "kind": line_spacing_str(self.line_spacing_kind),
                "value": self.line_spacing,
            },
            "indent": self.indent_pt,
            "padding": [self.padding_left_pt, self.padding_right_pt],
            "margin": [self.margin_top_pt, self.margin_bottom_pt],
            "heading": self.heading_level,
            "keepWithNext": self.keep_with_next,
            "pageBreakBefore": self.page_break_before,
        })
    }
}

fn align_str(a: Align) -> &'static str {
    match a {
        Align::Justify => "justify",
        Align::Left => "left",
        Align::Right => "right",
        Align::Center => "center",
        Align::Distribute => "distribute",
        Align::DistributeSpace => "distribute-space",
    }
}

fn line_spacing_str(k: LineSpacingKind) -> &'static str {
    match k {
        LineSpacingKind::Percent => "percent",
        LineSpacingKind::Fixed => "fixed",
        LineSpacingKind::BetweenLine => "between-line",
        LineSpacingKind::AtLeast => "at-least",
    }
}

/// HWP stores dimensional values in HWPUNIT (1 HWPUNIT = 1/7200 inch).
/// Convert to pt: 1 pt = 1/72 inch ⇒ 1 pt = 100 HWPUNIT.
pub fn hwpunit_to_pt(u: i32) -> f32 {
    u as f32 / 100.0
}
