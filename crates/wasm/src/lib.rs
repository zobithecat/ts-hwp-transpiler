//! WASM bridge. Thin serde-wasm-bindgen glue over core/codec/render.
//!
//! Exported JS API:
//!   loadHwp(bytes)                    → u32 doc handle
//!   loadHwpx(bytes)                   → u32 doc handle (ZIP-only)
//!   saveHwp(handle)                   → Uint8Array (.hwp OLE)
//!   saveHwpx(handle)                  → Uint8Array (.hwpx ZIP, stub)
//!   exportMarkdown(handle, …flags)    → Markdown string
//!   exportHtml(handle, …flags)        → HTML preview fragment
//!   renderPage(handle, pageIdx)       → RenderCommandList
//!   disposeDoc(handle)                → releases the doc from the
//!                                        wasm-side registry
//!   tokenizeFormula(script)           → token[]
//!   version()                         → string
//!
//! Handle model — an `IrDocument` stays resident on the wasm heap and
//! is addressed by a plain u32 handle on the JS side. Every other
//! call takes the handle, so nothing crosses the wasm boundary after
//! initial load. This is a hard requirement for real-world documents:
//! a 62 MB HWP with ~100 embedded binaries serialises into a multi-
//! hundred-megabyte JS object tree through `serde_wasm_bindgen`, and
//! that round-trips on every option-flip in the demo. Keeping the
//! document resident drops the per-render cost from O(doc_size) to
//! O(output_size).

use std::cell::RefCell;
use std::collections::HashMap;

use hwp_transpiler_codec::export::markdown::{self, LlmOptions, MdOptions};
use hwp_transpiler_codec::hwp::HwpReader;
use hwp_transpiler_codec::hwpx::skeleton::bundle_default_skeleton;
use hwp_transpiler_codec::hwpx::HwpxReader;
use hwp_transpiler_codec::import::markdown as md_import;
use hwp_transpiler_core::formula::Lexer;
use hwp_transpiler_core::ir::{IrDocument, Reader, Writer};
use hwp_transpiler_render::html::{self, HtmlOptions};
use hwp_transpiler_render::Renderer;
use wasm_bindgen::prelude::*;

thread_local! {
    static DOCS: RefCell<HashMap<u32, IrDocument>> = RefCell::new(HashMap::new());
    static NEXT_ID: RefCell<u32> = const { RefCell::new(1) };
}

fn insert_doc(doc: IrDocument) -> u32 {
    let id = NEXT_ID.with(|n| {
        let mut n = n.borrow_mut();
        let id = *n;
        *n = n.wrapping_add(1).max(1);
        id
    });
    DOCS.with(|d| {
        d.borrow_mut().insert(id, doc);
    });
    id
}

/// Look up a resident doc and run `f` against it. Any mutation or
/// return value is the caller's responsibility — most callers just
/// need read access.
fn with_doc<T>(handle: u32, f: impl FnOnce(&IrDocument) -> T) -> Result<T, JsValue> {
    DOCS.with(|d| {
        let d = d.borrow();
        let doc = d
            .get(&handle)
            .ok_or_else(|| JsValue::from_str("invalid doc handle"))?;
        Ok(f(doc))
    })
}

fn js_err<E: std::fmt::Display>(e: E) -> JsValue {
    JsValue::from_str(&e.to_string())
}

#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Magic-byte sniff so callers can feed either `.hwp` (OLE compound
/// file) or `.hwpx` (ZIP) without picking the right reader
/// themselves. Returns a handle into the wasm-side document registry.
#[wasm_bindgen(js_name = loadHwp)]
pub fn load_hwp(bytes: &[u8]) -> Result<u32, JsValue> {
    let doc = if bytes.starts_with(b"PK\x03\x04") {
        HwpxReader.read(bytes).map_err(js_err)?
    } else {
        HwpReader.read(bytes).map_err(js_err)?
    };
    Ok(insert_doc(doc))
}

/// Explicit HWPX-only entrypoint for callers that want to reject
/// anything that isn't OCF. Rejects HWP5 magic up front.
#[wasm_bindgen(js_name = loadHwpx)]
pub fn load_hwpx(bytes: &[u8]) -> Result<u32, JsValue> {
    if !bytes.starts_with(b"PK\x03\x04") {
        return Err(JsValue::from_str("loadHwpx: input is not a ZIP container"));
    }
    let doc = HwpxReader.read(bytes).map_err(js_err)?;
    Ok(insert_doc(doc))
}

/// Drop the document from the wasm-side registry, freeing its
/// memory. The demo calls this on every new file selection so
/// long-lived sessions don't accumulate garbage.
#[wasm_bindgen(js_name = disposeDoc)]
pub fn dispose_doc(handle: u32) {
    DOCS.with(|d| {
        d.borrow_mut().remove(&handle);
    });
}

#[wasm_bindgen(js_name = saveHwp)]
pub fn save_hwp(handle: u32) -> Result<Vec<u8>, JsValue> {
    with_doc(handle, |doc| doc.clone())?
        .then_emit_hwp()
}

#[wasm_bindgen(js_name = saveHwpx)]
pub fn save_hwpx(handle: u32) -> Result<Vec<u8>, JsValue> {
    with_doc(handle, |doc| doc.clone())?
        .then_emit_hwpx()
}

/// Run the Markdown exporter. Flags mirror the `hwp-to-md` CLI.
///
/// `assetsPath` is *not* taken — the browser doesn't have a sidecar
/// directory; image `![](…)` links should be rewritten by the caller
/// if they want to embed assets as Blob URLs.
///
/// `asset_mode` is an integer enum: `0 = None`, `1 = Inline`,
/// `2 = Split`. `Split` returns only the main MD here — call
/// `exportMarkdownAssets` to retrieve the companion string.
/// `asset_dpi` is the resampling DPI when an asset mode is active;
/// `0` falls back to the codec default (72).
#[wasm_bindgen(js_name = exportMarkdown)]
pub fn export_markdown(
    handle: u32,
    llm: bool,
    emit_roles: bool,
    emit_editable: bool,
    emit_domain_hints: bool,
    emit_styles: bool,
    asset_mode: u32,
    asset_dpi: u32,
) -> Result<String, JsValue> {
    let opts = build_md_opts(
        llm,
        emit_roles,
        emit_editable,
        emit_domain_hints,
        emit_styles,
        asset_mode,
        asset_dpi,
    );
    with_doc(handle, |doc| markdown::to_markdown_with(doc, &opts))
}

/// Companion text for `AssetMode::Split`. Returns an empty string
/// when split mode wasn't selected or the doc has no embedded
/// pictures.
#[wasm_bindgen(js_name = exportMarkdownAssets)]
pub fn export_markdown_assets(
    handle: u32,
    llm: bool,
    emit_roles: bool,
    emit_editable: bool,
    emit_domain_hints: bool,
    emit_styles: bool,
    asset_dpi: u32,
) -> Result<String, JsValue> {
    let opts = build_md_opts(
        llm,
        emit_roles,
        emit_editable,
        emit_domain_hints,
        emit_styles,
        2, // Split
        asset_dpi,
    );
    with_doc(handle, |doc| {
        markdown::to_markdown_export(doc, &opts)
            .assets
            .unwrap_or_default()
    })
}

fn build_md_opts(
    llm: bool,
    emit_roles: bool,
    emit_editable: bool,
    emit_domain_hints: bool,
    emit_styles: bool,
    asset_mode: u32,
    asset_dpi: u32,
) -> MdOptions {
    let mode = match asset_mode {
        1 => markdown::AssetMode::Inline,
        2 => markdown::AssetMode::Split,
        _ => markdown::AssetMode::None,
    };
    MdOptions {
        assets_path: None,
        llm: llm.then(|| LlmOptions {
            emit_roles,
            emit_editable,
            domain_hints: emit_domain_hints,
        }),
        domain_hints: emit_domain_hints && !llm,
        emit_roles: emit_roles && !llm,
        emit_editable: emit_editable && !llm,
        emit_styles: emit_styles && !llm,
        asset_mode: mode,
        asset_dpi: if asset_dpi == 0 { None } else { Some(asset_dpi) },
    }
}

/// Render to the browser-preview HTML fragment.
#[wasm_bindgen(js_name = exportHtml)]
pub fn export_html(
    handle: u32,
    assets_path: Option<String>,
    emit_styles: bool,
    emit_pages: bool,
) -> Result<String, JsValue> {
    let opts = HtmlOptions {
        assets_path,
        emit_styles,
        emit_pages,
    };
    with_doc(handle, |doc| html::to_html_with(doc, &opts))
}

/// Parse a UTF-8 Markdown string into an `IrDocument`, bundle the
/// minimum-viable HWPX skeleton (META-INF/container.xml,
/// Contents/content.hpf, Contents/header.xml) into
/// `unknown_streams`, and return a wasm-side handle. Pair with
/// `saveHwpx(handle)` for the MD → HWPX leg of the round-trip.
#[wasm_bindgen(js_name = importMarkdown)]
pub fn import_markdown(md: &str) -> Result<u32, JsValue> {
    let mut doc = md_import::from_markdown(md).map_err(js_err)?;
    bundle_default_skeleton(&mut doc);
    Ok(insert_doc(doc))
}

#[wasm_bindgen(js_name = renderPage)]
pub fn render_page(handle: u32, page: usize) -> Result<JsValue, JsValue> {
    let cmds = with_doc(handle, |doc| {
        let mut r = hwp_transpiler_render::HwpRenderer::default();
        r.render_page(doc, page)
    })?;
    serde_wasm_bindgen::to_value(&cmds).map_err(js_err)
}

/// Lightweight summary of the resident IR for browser-side console
/// debugging. Returns a JSON string so the caller can `console.log`
/// it without serde-wasm-bindgen overhead. Field shape:
///   { sections: N, paragraphs: M, pictures: P, tables: T,
///     bin_data: [{ id, mime, bytes }] }
#[wasm_bindgen(js_name = inspectIr)]
pub fn inspect_ir(handle: u32) -> Result<String, JsValue> {
    use hwp_transpiler_core::ir::ControlKind;
    with_doc(handle, |doc| {
        let mut paragraphs = 0usize;
        let mut pictures = 0usize;
        let mut tables = 0usize;
        for section in &doc.sections {
            paragraphs += section.paragraphs.len();
            for para in &section.paragraphs {
                for ctrl in &para.controls {
                    match &ctrl.kind {
                        ControlKind::Picture(_) => pictures += 1,
                        ControlKind::Table(_) => tables += 1,
                        _ => {}
                    }
                }
            }
        }
        let mut bin_summary = String::new();
        bin_summary.push('[');
        for (i, entry) in doc.bin_data.iter().enumerate() {
            if i > 0 {
                bin_summary.push(',');
            }
            let mime = entry.mime.as_deref().unwrap_or("");
            bin_summary.push_str(&format!(
                r#"{{"id":"{id}","mime":"{mime}","bytes":{n}}}"#,
                id = json_escape(&entry.id),
                mime = json_escape(mime),
                n = entry.bytes.len(),
            ));
        }
        bin_summary.push(']');
        format!(
            r#"{{"sections":{s},"paragraphs":{p},"pictures":{pic},"tables":{t},"bin_data":{b}}}"#,
            s = doc.sections.len(),
            p = paragraphs,
            pic = pictures,
            t = tables,
            b = bin_summary,
        )
    })
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
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

// ── IrDocument write helpers ────────────────────────────────────────

/// Small extension so the save_* functions can chain a clone → emit
/// without having to borrow the registry across the write call (the
/// writer takes `&IrDocument` so a short-lived clone is the
/// cheapest way to satisfy the borrow checker).
trait HwpEmit {
    fn then_emit_hwp(&self) -> Result<Vec<u8>, JsValue>;
    fn then_emit_hwpx(&self) -> Result<Vec<u8>, JsValue>;
}

impl HwpEmit for IrDocument {
    fn then_emit_hwp(&self) -> Result<Vec<u8>, JsValue> {
        let mut w = hwp_transpiler_codec::hwp::HwpWriter::default();
        w.write(self).map_err(js_err)
    }
    fn then_emit_hwpx(&self) -> Result<Vec<u8>, JsValue> {
        // Bundle the OCF skeleton parts (META-INF/container.xml,
        // Contents/content.hpf, Contents/header.xml, settings.xml,
        // version.xml) before writing — HWP5-sourced IRs don't carry
        // any of these, and the HWPX writer's path-prefix filter
        // strips the OLE leakage but doesn't synthesise the missing
        // HWPX-required parts. Idempotent: real HWPX-sourced IRs
        // already have these in `unknown_streams`, so the bundle
        // call is a no-op for them.
        let mut clone = self.clone();
        bundle_default_skeleton(&mut clone);
        let mut w = hwp_transpiler_codec::hwpx::HwpxWriter::default();
        w.write(&clone).map_err(js_err)
    }
}
