//! High-fidelity HWP page renderer.
//!
//! Output is a renderer-agnostic [`RenderCommandList`]. The WASM layer maps
//! commands to Canvas2D, SVG, or WebGL — whichever the host chose. Keeping
//! layout out of JS means the browser never touches pt→px conversion or
//! Hangul line-breaking.
//!
//! Subsystems (scaffolded):
//!   * [`layout`]   — page/paragraph/table layout (HWP box model → cmds)
//!   * [`commands`] — primitive draw-op set consumed by any backend

pub mod commands;
pub mod layout;

pub use commands::{RenderCommand, RenderCommandList, Rgba};

use hwp_transpiler_core::ir::IrDocument;

pub trait Renderer {
    fn page_count(&self, doc: &IrDocument) -> usize;
    fn render_page(&mut self, doc: &IrDocument, page: usize) -> RenderCommandList;
}

#[derive(Default)]
pub struct HwpRenderer;

impl Renderer for HwpRenderer {
    fn page_count(&self, _doc: &IrDocument) -> usize {
        0
    }
    fn render_page(&mut self, _doc: &IrDocument, _page: usize) -> RenderCommandList {
        RenderCommandList::default()
    }
}
