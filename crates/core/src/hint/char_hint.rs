//! CharShape → hint JSON. Mirrors `hwp::CharShape` structure but drops
//! never-rendered fields (emboss/engrave/kerning flags) and converts the
//! 1/100-pt HWP unit to plain pt on the emission boundary.

use super::Hintable;
use serde::Serialize;
use serde_json::{Value, json};

/// HWP CharShape carries 7 parallel script slots. Match this enum's order
/// to `hwp::CharShape::font_ids: [u16; 7]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ScriptKind {
    Hangul = 0,
    Latin = 1,
    Hanja = 2,
    Japanese = 3,
    Other = 4,
    Symbol = 5,
    User = 6,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScriptSlot {
    pub family: Option<String>,
    pub scale_pct: u8,
    pub spacing: i8,
    pub size_pct: u8,
    pub position: i8,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CharHint {
    /// 7 slots, same order as `ScriptKind` discriminants.
    pub scripts: [Option<ScriptSlot>; 7],
    /// HWP `base_size` in pt (converted from centipt on construction).
    pub size_pt: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub subscript: bool,
    pub superscript: bool,
    pub color_hex: String,
    pub underline_color_hex: Option<String>,
    pub shade_color_hex: Option<String>,
}

impl CharHint {
    pub const fn script_index(kind: ScriptKind) -> usize {
        kind as usize
    }

    pub fn hangul(&self) -> Option<&ScriptSlot> {
        self.scripts[Self::script_index(ScriptKind::Hangul)].as_ref()
    }
    pub fn latin(&self) -> Option<&ScriptSlot> {
        self.scripts[Self::script_index(ScriptKind::Latin)].as_ref()
    }
}

impl Hintable for CharHint {
    fn to_hint(&self) -> Value {
        let scripts: Value = self
            .scripts
            .iter()
            .map(|s| match s {
                Some(slot) => json!({
                    "family": slot.family,
                    "scale": slot.scale_pct,
                    "spacing": slot.spacing,
                    "size": slot.size_pct,
                    "position": slot.position,
                }),
                None => Value::Null,
            })
            .collect();

        json!({
            "kind": "char",
            "scripts": scripts,
            "size": self.size_pt,
            "weight": if self.bold { "bold" } else { "normal" },
            "style": if self.italic { "italic" } else { "normal" },
            "deco": {
                "underline": self.underline,
                "strike": self.strike,
                "sub": self.subscript,
                "sup": self.superscript,
            },
            "color": self.color_hex,
            "underlineColor": self.underline_color_hex,
            "shadeColor": self.shade_color_hex,
        })
    }
}

/// Helper for adapters: convert the raw HWP `base_size` (centi-pt) to pt.
pub fn centipt_to_pt(centipt: i32) -> f32 {
    centipt as f32 / 100.0
}
