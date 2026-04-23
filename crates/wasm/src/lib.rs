//! WASM bridge. Thin serde-wasm-bindgen glue over core/codec/render.
//!
//! Exported JS API:
//!   loadHwp(bytes)                    → IrDocument (as JS object)
//!   saveHwp(ir)                       → Uint8Array (.hwp binary, OLE)
//!   saveHwpx(ir)                      → Uint8Array (stub)
//!   exportMarkdown(ir, llm, roles,
//!                  editable, domain,
//!                  styles)            → Markdown string
//!   exportHtml(ir, assetsPath,
//!              styles, pages)         → HTML preview fragment
//!   renderPage(ir, pageIdx)           → RenderCommandList
//!   tokenizeFormula(script)           → token[]
//!   version()                         → string
//!
//! `loadHwpx` and `importMarkdown` remain stubs: HWPX read isn't
//! decoded yet, and md→hwp round-trip is a separate track.

use hwp_transpiler_codec::export::markdown::{self, LlmOptions, MdOptions};
use hwp_transpiler_codec::hwp::HwpReader;
use hwp_transpiler_codec::hwpx::HwpxReader;
use hwp_transpiler_core::formula::Lexer;
use hwp_transpiler_core::ir::{IrDocument, Reader, Writer};
use hwp_transpiler_render::html::{self, HtmlOptions};
use hwp_transpiler_render::Renderer;
use wasm_bindgen::prelude::*;

fn js_err<E: std::fmt::Display>(e: E) -> JsValue {
    JsValue::from_str(&e.to_string())
}

fn to_jsvalue(doc: &IrDocument) -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(doc).map_err(js_err)
}

fn from_jsvalue(doc: JsValue) -> Result<IrDocument, JsValue> {
    serde_wasm_bindgen::from_value(doc).map_err(js_err)
}

#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Dispatches by the bytes' container magic so callers can feed
/// either `.hwp` (OLE compound file) or `.hwpx` (ZIP) without
/// picking the right entrypoint themselves. Same sniff as the
/// `hwp-to-md` CLI.
#[wasm_bindgen(js_name = loadHwp)]
pub fn load_hwp(bytes: &[u8]) -> Result<JsValue, JsValue> {
    let doc = if bytes.starts_with(b"PK\x03\x04") {
        HwpxReader.read(bytes).map_err(js_err)?
    } else {
        HwpReader.read(bytes).map_err(js_err)?
    };
    to_jsvalue(&doc)
}

/// Explicit HWPX-only entrypoint for callers that want to refuse
/// anything that isn't OCF. Rejects HWP5 magic up front rather than
/// falling through.
#[wasm_bindgen(js_name = loadHwpx)]
pub fn load_hwpx(bytes: &[u8]) -> Result<JsValue, JsValue> {
    if !bytes.starts_with(b"PK\x03\x04") {
        return Err(JsValue::from_str("loadHwpx: input is not a ZIP container"));
    }
    let doc = HwpxReader.read(bytes).map_err(js_err)?;
    to_jsvalue(&doc)
}

#[wasm_bindgen(js_name = saveHwp)]
pub fn save_hwp(doc: JsValue) -> Result<Vec<u8>, JsValue> {
    let d = from_jsvalue(doc)?;
    let mut w = hwp_transpiler_codec::hwp::HwpWriter::default();
    w.write(&d).map_err(js_err)
}

#[wasm_bindgen(js_name = saveHwpx)]
pub fn save_hwpx(doc: JsValue) -> Result<Vec<u8>, JsValue> {
    let d = from_jsvalue(doc)?;
    let mut w = hwp_transpiler_codec::hwpx::HwpxWriter::default();
    w.write(&d).map_err(js_err)
}

/// Run the Markdown exporter on an IR loaded via [`load_hwp`]. Flags
/// mirror the `hwp-to-md` CLI one-for-one:
///
///   * `llm`             — switch to the structured LLM-friendly
///                         record format (`SECTION[id=…]` etc.)
///   * `emit_roles`      — tag cells with header/label/value/spacer
///   * `emit_editable`   — tag cells with editable=true|false|unknown
///   * `emit_domain_hints` — tag tables with kind=<domain>
///   * `emit_styles`     — inline bold/italic/strike wrappers (human
///                         path only; ignored under `llm`)
///
/// `assetsPath` is *not* taken here — the browser doesn't have a
/// sidecar directory; image `![](…)` links should be rewritten by the
/// caller if they plan to embed assets as Blob URLs.
#[wasm_bindgen(js_name = exportMarkdown)]
pub fn export_markdown(
    doc: JsValue,
    llm: bool,
    emit_roles: bool,
    emit_editable: bool,
    emit_domain_hints: bool,
    emit_styles: bool,
) -> Result<String, JsValue> {
    let d = from_jsvalue(doc)?;
    let opts = MdOptions {
        assets_path: None,
        llm: llm.then(|| LlmOptions {
            emit_roles,
            emit_editable,
            domain_hints: emit_domain_hints,
        }),
        // Mirror flags apply only when llm is off (the LLM path has
        // its own LlmOptions field for the same knobs).
        domain_hints: emit_domain_hints && !llm,
        emit_roles: emit_roles && !llm,
        emit_editable: emit_editable && !llm,
        emit_styles: emit_styles && !llm,
    };
    Ok(markdown::to_markdown_with(&d, &opts))
}

/// Render an IR to the browser-preview HTML fragment.
///
///   * `assetsPath`   — URL prefix for `<img>` sources (typically
///                      a blob: URL prefix or a static hosting path).
///                      Pass `None` to omit images entirely.
///   * `emit_styles`  — wrap CharShape runs with `<strong>/<em>/<s>`
///                      plus a `<span style="…">` when colour / size /
///                      font differ from HWP defaults.
///   * `emit_pages`   — wrap each section with
///                      `<section class="hwp-page" style="width:…mm;
///                      height:…mm;padding:…">` from PAGE_DEF.
#[wasm_bindgen(js_name = exportHtml)]
pub fn export_html(
    doc: JsValue,
    assets_path: Option<String>,
    emit_styles: bool,
    emit_pages: bool,
) -> Result<String, JsValue> {
    let d = from_jsvalue(doc)?;
    let opts = HtmlOptions {
        assets_path,
        emit_styles,
        emit_pages,
    };
    Ok(html::to_html_with(&d, &opts))
}

#[wasm_bindgen(js_name = importMarkdown)]
pub fn import_markdown(_md: &str) -> Result<JsValue, JsValue> {
    Err(JsValue::from_str("importMarkdown: not yet implemented"))
}

#[wasm_bindgen(js_name = renderPage)]
pub fn render_page(doc: JsValue, page: usize) -> Result<JsValue, JsValue> {
    let d = from_jsvalue(doc)?;
    let mut r = hwp_transpiler_render::HwpRenderer::default();
    let cmds = r.render_page(&d, page);
    serde_wasm_bindgen::to_value(&cmds).map_err(js_err)
}

#[wasm_bindgen(js_name = tokenizeFormula)]
pub fn tokenize_formula(script: &str) -> Result<JsValue, JsValue> {
    let tokens = Lexer::new(script).tokenize();
    let names: Vec<String> = tokens
        .into_iter()
        .map(|t| format!("{:?}", t.kind))
        .collect();
    serde_wasm_bindgen::to_value(&names).map_err(js_err)
}
