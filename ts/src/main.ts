/// <reference types="vite/client" />
// Browser demo for ts-hwp-transpiler.
//
// Split design:
//   * LEFT pane — full HWP editor iframe from `@rhwp/editor` (3-line
//     embed of rhwp-studio). Menu, toolbar, page rendering, editing —
//     all hosted inside the iframe; we just call `editor.loadFile()`
//     with the user-selected bytes.
//   * RIGHT pane — Markdown output from our own wasm `exportMarkdown`.
//     Structured MD export (stable ids, role/editable, domain hints)
//     is the differentiator, so that's what lives here.
//
// Both engines parse the same bytes independently. The editor iframe
// has its own wasm; our wasm is co-located next to main.ts.

import { createEditor } from "@rhwp/editor";

import init, {
  loadHwp,
  exportHtml,
  exportMarkdown,
  version,
} from "./wasm/hwp_transpiler_wasm.js";

const $ = <T extends HTMLElement = HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

const fileInput = $<HTMLInputElement>("file");
const statusEl = $("status");
const previewEl = $("preview");
const mdPreviewEl = $("md-preview");
const previewMeta = $("preview-meta");
const markdownEl = $<HTMLPreElement>("markdown");
const copyBtn = $<HTMLButtonElement>("copy-md");
const tabButtons = document.querySelectorAll<HTMLButtonElement>(
  ".tabs .tab",
);

const llmModeEl = $<HTMLInputElement>("llm-mode");
const emitRolesEl = $<HTMLInputElement>("emit-roles");
const domainHintsEl = $<HTMLInputElement>("domain-hints");
const emitStylesEl = $<HTMLInputElement>("emit-styles");

// Resident IR. We don't re-parse on option changes — just flip the
// knobs against the cached IR.
let ourIr: unknown = null;

// Currently-visible left-pane tab. The editor iframe and our
// structure-preserving HTML render coexist as sibling divs;
// switching toggles `hidden` without tearing down either.
type LeftTab = "editor" | "html";
let activeTab: LeftTab = "editor";

// rhwp-editor handle; populated by createEditor() at boot.
let editor: Awaited<ReturnType<typeof createEditor>> | null = null;

function setStatus(text: string, isError = false): void {
  statusEl.textContent = text;
  statusEl.classList.toggle("error", isError);
}

function renderMarkdown(ir: unknown): { bytes: number; ms: number } {
  const started = performance.now();
  const md = exportMarkdown(
    ir,
    llmModeEl.checked,
    emitRolesEl.checked,
    // editable tagging rides with roles — the CLI emits them as a
    // pair in practice, and splitting the demo checkbox would just
    // add a knob nobody touches.
    emitRolesEl.checked,
    domainHintsEl.checked,
    emitStylesEl.checked,
  );
  markdownEl.textContent = md;
  copyBtn.disabled = md.length === 0;
  if (activeTab === "html") {
    renderStructuredHtml();
  }
  return { bytes: md.length, ms: Math.round(performance.now() - started) };
}

/// IR → structure-preserving HTML (real `<table rowspan colspan>`,
/// `<figure>/<figcaption>`, heading decoding). The wasm exporter
/// runs on the resident `ourIr` directly, so the render is
/// independent of whatever shape the MD pane is currently showing —
/// LLM-mode records render the same as human-mode bullets because
/// both originate from the same parsed IR.
function renderStructuredHtml(): void {
  if (!ourIr) {
    mdPreviewEl.innerHTML =
      '<p class="hint">파일을 선택하면 여기에 HTML 미리보기가 나타납니다.</p>';
    return;
  }
  try {
    // assets_path left undefined — we don't have image blob URLs in
    // this pane (the editor iframe handles images). emit_styles on
    // so bold/italic CharShape runs show up; emit_pages off because
    // real mm dimensions overflow the side pane.
    const html = exportHtml(ourIr, undefined, emitStylesEl.checked, false);
    mdPreviewEl.innerHTML = html;
  } catch (err) {
    mdPreviewEl.textContent = `exportHtml 실패: ${String(err)}`;
  }
}

function setTab(target: LeftTab): void {
  activeTab = target;
  tabButtons.forEach((btn) => {
    btn.classList.toggle("is-active", btn.dataset.target === target);
  });
  previewEl.hidden = target !== "editor";
  mdPreviewEl.hidden = target !== "html";
  if (target === "html") {
    renderStructuredHtml();
  }
}

async function loadIntoEditor(
  buffer: ArrayBuffer,
  fileName: string,
): Promise<number> {
  if (!editor) throw new Error("editor not ready");
  const result = await editor.loadFile(buffer, fileName);
  return result?.pageCount ?? 0;
}

async function handleFile(file: File): Promise<void> {
  setStatus(`읽는 중… ${file.name}`);
  const buffer = await file.arrayBuffer();
  const bytes = new Uint8Array(buffer);

  // Kick off both paths in parallel — the editor iframe load and our
  // wasm parse are completely independent.
  const started = performance.now();
  const [pagesResult, irResult] = await Promise.allSettled([
    loadIntoEditor(buffer, file.name),
    Promise.resolve().then(() => loadHwp(bytes)),
  ]);

  if (pagesResult.status === "fulfilled") {
    previewMeta.textContent = `${pagesResult.value} pages`;
  } else {
    previewMeta.textContent = "preview load failed";
  }

  if (irResult.status === "fulfilled") {
    ourIr = irResult.value;
    const m = renderMarkdown(ourIr);
    const ms = Math.round(performance.now() - started);
    setStatus(`loaded in ${ms}ms · MD ${m.bytes.toLocaleString()} bytes`);
  } else {
    ourIr = null;
    markdownEl.textContent = "";
    copyBtn.disabled = true;
    setStatus(`loadHwp 실패: ${String(irResult.reason)}`, true);
  }
}

fileInput.addEventListener("change", () => {
  const f = fileInput.files?.[0];
  if (f) void handleFile(f);
});

document.body.addEventListener("dragover", (e) => e.preventDefault());
document.body.addEventListener("drop", (e) => {
  e.preventDefault();
  const f = e.dataTransfer?.files?.[0];
  if (f) void handleFile(f);
});

for (const el of [llmModeEl, emitRolesEl, domainHintsEl, emitStylesEl]) {
  el.addEventListener("change", () => {
    if (!ourIr) return;
    const m = renderMarkdown(ourIr);
    setStatus(`MD ${m.bytes.toLocaleString()} bytes / ${m.ms}ms`);
  });
}

tabButtons.forEach((btn) => {
  btn.addEventListener("click", () => {
    const target = btn.dataset.target as LeftTab | undefined;
    if (target) setTab(target);
  });
});

copyBtn.addEventListener("click", async () => {
  const text = markdownEl.textContent ?? "";
  if (!text) return;
  try {
    await navigator.clipboard.writeText(text);
    copyBtn.textContent = "복사됨";
    setTimeout(() => (copyBtn.textContent = "복사"), 1200);
  } catch (err) {
    setStatus(`복사 실패: ${String(err)}`, true);
  }
});

// Boot in parallel: our wasm self-initialises from its sidecar file;
// the editor mounts into the preview pane and spins up its own iframe
// against the default rhwp-studio URL.
await Promise.all([
  init(),
  (async () => {
    // Clear the static "pick a file" placeholder before the iframe mounts.
    previewEl.innerHTML = "";
    editor = await createEditor(previewEl);
  })(),
]);
setStatus(`ready · ts-hwp-transpiler ${version()} · @rhwp/editor hosted`);
