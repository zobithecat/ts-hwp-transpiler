//! WASM bridge. Thin serde-wasm-bindgen glue over core/writer/render.
//!
//! Exported JS API:
//!   loadHwp(bytes)   → IrDocument
//!   loadHwpx(bytes)  → IrDocument
//!   saveHwp(ir)      → Uint8Array   (.hwp binary, OLE)
//!   saveHwpx(ir)     → Uint8Array   (.hwpx, ZIP+XML)
//!   exportMarkdown(ir) → string     (with hinting layer)
//!   importMarkdown(md) → IrDocument
//!   renderPage(ir, pageIdx) → RenderCommandList
//!   tokenizeFormula(script) → Token[]
//!   version() → string

use hwp_transpiler_core::formula::Lexer;
use hwp_transpiler_core::ir::{IrDocument, Writer};
use hwp_transpiler_render::Renderer;
use wasm_bindgen::prelude::*;

fn js_err<E: std::fmt::Display>(e: E) -> JsValue {
    JsValue::from_str(&e.to_string())
}

#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[wasm_bindgen(js_name = loadHwp)]
pub fn load_hwp(_bytes: &[u8]) -> Result<JsValue, JsValue> {
    Err(JsValue::from_str("loadHwp: not yet implemented"))
}

#[wasm_bindgen(js_name = loadHwpx)]
pub fn load_hwpx(_bytes: &[u8]) -> Result<JsValue, JsValue> {
    Err(JsValue::from_str("loadHwpx: not yet implemented"))
}

#[wasm_bindgen(js_name = saveHwp)]
pub fn save_hwp(doc: JsValue) -> Result<Vec<u8>, JsValue> {
    let d: IrDocument = serde_wasm_bindgen::from_value(doc).map_err(js_err)?;
    let mut w = hwp_transpiler_codec::hwp::HwpWriter::default();
    w.write(&d).map_err(js_err)
}

#[wasm_bindgen(js_name = saveHwpx)]
pub fn save_hwpx(doc: JsValue) -> Result<Vec<u8>, JsValue> {
    let d: IrDocument = serde_wasm_bindgen::from_value(doc).map_err(js_err)?;
    let mut w = hwp_transpiler_codec::hwpx::HwpxWriter::default();
    w.write(&d).map_err(js_err)
}

#[wasm_bindgen(js_name = exportMarkdown)]
pub fn export_markdown(_doc: JsValue) -> Result<String, JsValue> {
    Err(JsValue::from_str("exportMarkdown: not yet implemented"))
}

#[wasm_bindgen(js_name = importMarkdown)]
pub fn import_markdown(_md: &str) -> Result<JsValue, JsValue> {
    Err(JsValue::from_str("importMarkdown: not yet implemented"))
}

#[wasm_bindgen(js_name = renderPage)]
pub fn render_page(doc: JsValue, page: usize) -> Result<JsValue, JsValue> {
    let d: IrDocument = serde_wasm_bindgen::from_value(doc).map_err(js_err)?;
    let mut r = hwp_transpiler_render::HwpRenderer::default();
    let cmds = r.render_page(&d, page);
    serde_wasm_bindgen::to_value(&cmds).map_err(js_err)
}

#[wasm_bindgen(js_name = tokenizeFormula)]
pub fn tokenize_formula(script: &str) -> Result<JsValue, JsValue> {
    let tokens = Lexer::new(script).tokenize();
    let names: Vec<String> = tokens.into_iter().map(|t| format!("{:?}", t.kind)).collect();
    serde_wasm_bindgen::to_value(&names).map_err(js_err)
}
