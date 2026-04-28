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
  disposeDoc,
  exportHtml,
  exportMarkdown,
  importMarkdown,
  loadHwp,
  saveHwpx,
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
const pdfBtn = $<HTMLButtonElement>("pdf-download");
const htmlBtn = $<HTMLButtonElement>("html-download");
const mdDlBtn = $<HTMLButtonElement>("md-download");
const hwpxDlBtn = $<HTMLButtonElement>("hwpx-download");
const tabButtons = document.querySelectorAll<HTMLButtonElement>(
  ".tabs .tab",
);

const llmModeEl = $<HTMLInputElement>("llm-mode");
const emitRolesEl = $<HTMLInputElement>("emit-roles");
const domainHintsEl = $<HTMLInputElement>("domain-hints");
const emitStylesEl = $<HTMLInputElement>("emit-styles");

// Handle into the wasm-side document registry. We flip options
// against this handle rather than re-parsing bytes; dispose it
// before loading a new file so the wasm heap doesn't accumulate
// abandoned docs.
let ourIr: number | null = null;

// Filename stem of the most recently loaded file; used to name the
// download outputs (`.md`, `.pdf`) after their source document.
let currentStem = "document";

// Currently-visible left-pane tab. The editor iframe and our
// structure-preserving HTML render coexist as sibling divs;
// switching toggles `hidden` without tearing down either. HTML is
// the default tab so users see our structured render first; the
// rhwp editor iframe only boots when the user explicitly switches
// to it.
type LeftTab = "editor" | "html";
let activeTab: LeftTab = "html";

// rhwp-editor handle — created lazily on the first editor-tab
// click. This keeps the initial page weight down (no cross-origin
// iframe, no 3 MB rhwp wasm fetch) until the user actually wants
// the editor surface. `editorInitPromise` caches the in-flight
// createEditor call so double-clicks don't race.
let editor: Awaited<ReturnType<typeof createEditor>> | null = null;
let editorInitPromise: Promise<void> | null = null;

// Most-recently loaded file bytes. Stored so the editor tab can
// deferred-load them when the user first clicks over.
let lastBuffer: ArrayBuffer | null = null;
let lastFileName: string | null = null;
// Tracks whether the currently-loaded buffer has reached the
// editor iframe yet — set after a successful `editor.loadFile()`.
let lastBufferLoadedInEditor = false;

function setStatus(text: string, isError = false): void {
  statusEl.textContent = text;
  statusEl.classList.toggle("error", isError);
}

function renderMarkdown(handle: number): { bytes: number; ms: number } {
  const started = performance.now();
  const md = exportMarkdown(
    handle,
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
  mdDlBtn.disabled = md.length === 0;
  hwpxDlBtn.disabled = md.length === 0;
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
  if (ourIr === null) {
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
  // PDF download only makes sense off the structured HTML render —
  // the rhwp iframe is cross-origin and can't be snapshotted from
  // the parent frame.
  pdfBtn.disabled = target !== "html" || ourIr === null;
  htmlBtn.disabled = target !== "html" || ourIr === null;
  if (target === "html") {
    renderStructuredHtml();
  } else if (target === "editor") {
    void ensureEditorLoaded();
  }
}

/// Boot the rhwp-editor iframe on demand, then hand it the most-
/// recently loaded bytes. Subsequent calls no-op until a new file
/// arrives. All state transitions funnel through the shared
/// `editorInitPromise` so concurrent tab clicks don't spawn two
/// iframes.
async function ensureEditorLoaded(): Promise<void> {
  if (editor === null && editorInitPromise === null) {
    editorInitPromise = (async () => {
      // Clear the "iframe 로드 중…" placeholder before the iframe mounts.
      previewEl.innerHTML = "";
      editor = await createEditor(previewEl);
    })();
  }
  if (editorInitPromise) {
    try {
      await editorInitPromise;
    } catch (err) {
      editorInitPromise = null;
      setStatus(`에디터 로드 실패: ${String(err)}`, true);
      return;
    }
  }
  if (!editor) return;
  if (lastBuffer && !lastBufferLoadedInEditor && lastFileName) {
    try {
      const result = await editor.loadFile(lastBuffer, lastFileName);
      const pages = result?.pageCount ?? 0;
      previewMeta.textContent = `${pages} pages`;
      lastBufferLoadedInEditor = true;
    } catch (err) {
      previewMeta.textContent = "preview load failed";
      setStatus(`에디터 파일 로드 실패: ${String(err)}`, true);
    }
  }
}

async function handleFile(file: File): Promise<void> {
  setStatus(`읽는 중… ${file.name}`);
  currentStem = stemOf(file.name);

  // Free the previous resident doc in the wasm registry before
  // parsing the new one. Large documents (62 MB+ HWP with embedded
  // images) can accumulate into hundreds of MB if we skip this.
  if (ourIr !== null) {
    disposeDoc(ourIr);
    ourIr = null;
  }

  if (isMarkdownFile(file.name)) {
    await handleMarkdownFile(file);
    return;
  }

  const buffer = await file.arrayBuffer();
  const bytes = new Uint8Array(buffer);

  // Remember the buffer for the editor iframe's deferred load.
  // The editor is only booted when the user switches to that tab,
  // so we can't push bytes into it here.
  lastBuffer = buffer;
  lastFileName = file.name;
  lastBufferLoadedInEditor = false;

  // Parse into our wasm immediately — the HTML / MD panes need a
  // resident IR to render against.
  const started = performance.now();
  let irResult: PromiseSettledResult<number>;
  try {
    irResult = { status: "fulfilled", value: loadHwp(bytes) };
  } catch (err) {
    irResult = { status: "rejected", reason: err };
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
    mdDlBtn.disabled = true;
    hwpxDlBtn.disabled = true;
    setStatus(`loadHwp 실패: ${String(irResult.reason)}`, true);
  }
  pdfBtn.disabled = activeTab !== "html" || ourIr === null;
  htmlBtn.disabled = activeTab !== "html" || ourIr === null;

  // If the user is already on the editor tab, deferred-load now so
  // they don't have to tab-click to retrigger; if they're still on
  // HTML, leave the editor alone — ensureEditorLoaded() runs when
  // they switch.
  if (activeTab === "editor") {
    void ensureEditorLoaded();
  }
}

/// `.md` / `.markdown` upload path. Reads as UTF-8 text, runs through
/// `importMarkdown`, and registers the resulting IR exactly like a
/// HWP/HWPX load — `ourIr` points at the imported doc so all three
/// panes (Markdown / HTML preview / .hwpx download) work against it.
/// The rhwp-editor iframe stays empty for this path because we don't
/// have HWP bytes to feed it; clicking the .hwpx download button
/// produces those after the fact.
async function handleMarkdownFile(file: File): Promise<void> {
  const text = await file.text();
  // Editor iframe has nothing to consume from a .md upload, so
  // clear any leftover buffer state from a previous HWP load.
  lastBuffer = null;
  lastFileName = null;
  lastBufferLoadedInEditor = false;

  const started = performance.now();
  let importedHandle: number;
  try {
    importedHandle = importMarkdown(text);
  } catch (err) {
    ourIr = null;
    markdownEl.textContent = "";
    copyBtn.disabled = true;
    mdDlBtn.disabled = true;
    hwpxDlBtn.disabled = true;
    pdfBtn.disabled = true;
    htmlBtn.disabled = true;
    setStatus(`importMarkdown 실패: ${String(err)}`, true);
    return;
  }

  ourIr = importedHandle;
  // Show the *original* uploaded Markdown in the right pane so the
  // user sees what they fed in. Re-exporting through `exportMarkdown`
  // would round-trip-and-lose-fidelity, which is misleading at upload
  // time. The HTML preview pane below renders our IR via exportHtml
  // so the user can sanity-check the import did the right thing.
  markdownEl.textContent = text;
  copyBtn.disabled = text.length === 0;
  mdDlBtn.disabled = text.length === 0;
  hwpxDlBtn.disabled = text.length === 0;
  if (activeTab === "html") {
    renderStructuredHtml();
  }
  pdfBtn.disabled = activeTab !== "html";
  htmlBtn.disabled = activeTab !== "html";
  const ms = Math.round(performance.now() - started);
  setStatus(`loaded .md in ${ms}ms · ${text.length.toLocaleString()} chars`);
}

function isMarkdownFile(name: string): boolean {
  const lower = name.toLowerCase();
  return lower.endsWith(".md") || lower.endsWith(".markdown");
}

/// Strip the directory path and trailing extension from a filename —
/// `"folder/plan.hwpx"` → `"plan"`. Used as the stem for download
/// filenames (`plan.md`, `plan.pdf`).
function stemOf(name: string): string {
  const base = name.split(/[\\/]/).pop() ?? name;
  const dot = base.lastIndexOf(".");
  return dot > 0 ? base.slice(0, dot) : base;
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

mdDlBtn.addEventListener("click", () => {
  const text = markdownEl.textContent ?? "";
  if (!text) return;
  downloadBlob(
    new Blob([text], { type: "text/markdown;charset=utf-8" }),
    `${currentStem}.md`,
  );
});

/// Save the resident IR as `.hwpx`. Two upload paths converge here:
///   * `.hwp` / `.hwpx` upload → `loadHwp` populates ourIr with the
///     original header / bin_data / unknown_streams. Saving keeps
///     images and original DocInfo verbatim.
///   * `.md` upload → `importMarkdown` populates ourIr from the MD
///     text plus `bundle_default_skeleton`. Saving uses our
///     synthesised header.
///
/// Going through MD again here would round-trip-and-lose: a HWPX
/// load's images live in `bin_data` (not in the Markdown text), so
/// re-importing the MD strips them. The dedicated round-trip
/// behaviour can be exposed later as a separate button if needed.
hwpxDlBtn.addEventListener("click", () => {
  if (ourIr === null) return;
  try {
    const started = performance.now();
    const bytes = saveHwpx(ourIr);
    const ms = Math.round(performance.now() - started);
    downloadBlob(
      // wasm-bindgen types `Uint8Array.buffer` as `ArrayBufferLike`,
      // which TS won't widen to `BlobPart`. Cast through `BlobPart`
      // — the runtime accepts Uint8Array directly.
      new Blob([bytes as BlobPart], { type: "application/hwp+zip" }),
      `${currentStem}.hwpx`,
    );
    setStatus(
      `HWPX ${bytes.length.toLocaleString()} bytes / ${ms}ms`,
    );
  } catch (err) {
    setStatus(`.hwpx 저장 실패: ${String(err)}`, true);
  }
});

/// Save the structured HTML render as a standalone `.html` document.
/// Uses the same print-ready shell as the PDF path so the file
/// stands on its own when opened in a browser — fonts via CDN, A4
/// page widths, table borders all baked in.
htmlBtn.addEventListener("click", () => {
  if (!ourIr || activeTab !== "html") return;
  const body = exportHtml(ourIr, undefined, emitStylesEl.checked, true);
  const html = printableHtmlShell(currentStem, body);
  downloadBlob(
    new Blob([html], { type: "text/html;charset=utf-8" }),
    `${currentStem}.html`,
  );
});

/// Open a print-ready popup with our structured HTML render and
/// trigger the browser's print dialog. The user chooses "Save as PDF"
/// from there — no client-side PDF library needed, no server-side
/// rendering. Popup-blocker gotcha: the window.open() has to run
/// synchronously inside the click handler or the browser treats it
/// as non-user-initiated and blocks it.
pdfBtn.addEventListener("click", () => {
  if (!ourIr || activeTab !== "html") return;
  const popup = window.open("", "_blank", "width=900,height=1100");
  if (!popup) {
    setStatus("팝업이 차단되어 PDF 프린트를 열 수 없습니다.", true);
    return;
  }
  // emit_pages on → real A4 widths so the print dialog's page
  // boundaries line up with the document's declared page size.
  const body = exportHtml(ourIr, undefined, emitStylesEl.checked, true);
  popup.document.write(printableHtmlShell(currentStem, body));
  popup.document.close();
  // Wait for layout before asking for print — Safari otherwise
  // shows an empty preview.
  popup.addEventListener("load", () => popup.print());
});

/// Wrap the exportHtml body in a minimal shell with print-oriented
/// CSS so A4 page breaks align with our `.hwp-page` sections.
function printableHtmlShell(title: string, body: string): string {
  return `<!DOCTYPE html>
<html lang="ko"><head>
<meta charset="UTF-8">
<title>${escapeHtml(title)}</title>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Noto+Sans+KR:wght@400;700&family=Noto+Serif+KR:wght@400;700&display=swap">
<style>
  body { margin: 0; font-family: "Noto Sans KR", sans-serif; color: #111; font-size: 11pt; line-height: 1.5; }
  .hwp-preview { max-width: none; }
  .hwp-page { margin: 0 auto 6mm; background: white; box-shadow: 0 1px 4px rgba(0,0,0,.08); box-sizing: border-box; }
  @media print {
    .hwp-page { margin: 0; box-shadow: none; page-break-after: always; }
  }
  table { border-collapse: collapse; margin: 4pt 0; }
  th, td { border: 0.3pt solid #999; padding: 3pt 5pt; vertical-align: top; }
  figure { margin: 6pt 0; text-align: center; }
  figure img { max-width: 100%; height: auto; }
  figcaption { font-size: 9pt; color: #555; margin-top: 2pt; }
  h1, h2, h3, h4 { margin: 10pt 0 4pt; }
</style>
</head><body>${body}</body></html>`;
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

/// Generic "download a Blob as a file" shim. Uses a temporary <a>
/// and an object URL; revokes the URL after click fires so long-
/// running sessions don't leak blob references.
function downloadBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}

// Align initial tab state with `activeTab` default ("html"). The
// HTML `is-active` class in `index.html` can lag behind; force-sync
// here so the visuals match regardless.
setTab(activeTab);

// Boot: only our wasm. The editor iframe stays dark until the user
// clicks its tab — no eager cross-origin fetch, no 3 MB of rhwp
// wasm pulled when the user only wants Markdown.
await init();
setStatus(`ready · ts-hwp-transpiler ${version()}`);
