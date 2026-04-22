use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RenderCommand {
    SetFont {
        family: String,
        size_pt: f32,
        bold: bool,
        italic: bool,
    },
    SetFill(Rgba),
    SetStroke {
        color: Rgba,
        width_pt: f32,
    },
    Text {
        x_pt: f32,
        y_pt: f32,
        text: String,
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
    },
    Rect {
        x_pt: f32,
        y_pt: f32,
        w_pt: f32,
        h_pt: f32,
        filled: bool,
    },
    Image {
        x_pt: f32,
        y_pt: f32,
        w_pt: f32,
        h_pt: f32,
        bin_id: String,
    },
    PushClip {
        x_pt: f32,
        y_pt: f32,
        w_pt: f32,
        h_pt: f32,
    },
    PopClip,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RenderCommandList {
    pub width_pt: f32,
    pub height_pt: f32,
    pub ops: Vec<RenderCommand>,
}
