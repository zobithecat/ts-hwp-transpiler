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
  exportMarkdownAssets,
  importMarkdown,
  inspectIr,
  loadHwp,
  saveHwpx,
  version,
} from "./wasm/hwp_transpiler_wasm.js";

/// Console-side debug prefix so the user can grep the dev console
/// for the demo's own log lines.
const TAG = "%c[hwp-demo]";
const TAG_STYLE = "color:#0a84ff;font-weight:bold";

function logIr(label: string, handle: number): void {
  try {
    const s = inspectIr(handle);
    const summary = JSON.parse(s);
    console.log(TAG, TAG_STYLE, label, summary);
  } catch (err) {
    console.warn(TAG, TAG_STYLE, label, "inspect failed:", err);
  }
}

/// Inspect the bytes a `saveHwpx` produced — list the entry names
/// inside the OCF / ZIP container without needing a full unzip
/// library. Reads the central directory directly. Best-effort,
/// silently bails on any malformed structure.
function logHwpxEntries(label: string, bytes: Uint8Array): void {
  try {
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const entries: { name: string; size: number }[] = [];
    // Scan for "PK\x01\x02" central directory headers.
    for (let i = 0; i < view.byteLength - 4; i++) {
      if (
        view.getUint8(i) === 0x50 &&
        view.getUint8(i + 1) === 0x4b &&
        view.getUint8(i + 2) === 0x01 &&
        view.getUint8(i + 3) === 0x02
      ) {
        const size = view.getUint32(i + 24, true);
        const nameLen = view.getUint16(i + 28, true);
        const nameStart = i + 46;
        const name = new TextDecoder().decode(
          bytes.subarray(nameStart, nameStart + nameLen),
        );
        entries.push({ name, size });
        i = nameStart + nameLen - 1;
      }
    }
    console.log(
      TAG,
      TAG_STYLE,
      label,
      `${bytes.length.toLocaleString()} bytes / ${entries.length} entries`,
      entries,
    );
  } catch (err) {
    console.warn(TAG, TAG_STYLE, label, "zip inspect failed:", err);
  }
}

const $ = <T extends HTMLElement = HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

const fileChipEl = $("file-chip");
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
const assetModeEl = $<HTMLSelectElement>("asset-mode");
const assetDpiEl = $<HTMLSelectElement>("asset-dpi");

/// 0 = None, 1 = Inline, 2 = Split — matches the wasm-side enum.
function assetModeInt(): number {
  switch (assetModeEl.value) {
    case "inline":
      return 1;
    case "split":
      return 2;
    default:
      return 0;
  }
}

function assetDpiInt(): number {
  // `0` is the "원본(무손실)" choice → wasm treats it as verbatim
  // (no re-encode), keeping the source's original image bytes/format
  // (e.g. BMP) so older viewers like 한글2018 render them. A plain
  // `|| 72` would wrongly coerce that 0 back to 72, so handle NaN
  // explicitly instead.
  const v = parseInt(assetDpiEl.value, 10);
  return Number.isNaN(v) ? 0 : v;
}

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
type LeftTab = "editor" | "html" | "markdown";
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
  // Mirror to the mobile floating bar — visible only under the mobile
  // media query but always present in the DOM.
  const mobileStatus = document.getElementById("mobile-status");
  if (mobileStatus) {
    mobileStatus.textContent = text;
    mobileStatus.classList.toggle("error", isError);
  }
}

/// What kind of source the loaded IR came from. Drives the colored
/// badge in the file chip so the user can tell at a glance whether
/// the right-pane outputs are roundtrip-back-to-binary or
/// from-scratch-via-MD scenarios.
type SourceKind = "hwp" | "hwpx" | "md" | "md+assets";

interface LoadedFile {
  kind: SourceKind;
  name: string;
  size: number;
}

function renderFileChip(loaded: LoadedFile | null): void {
  if (loaded === null) {
    fileChipEl.classList.remove("loaded");
    fileChipEl.innerHTML = `
      <label class="file-picker">
        <input type="file" id="file" accept=".hwp,.hwpx,.md,.markdown" multiple />
        <span class="file-picker-cta">📂 .hwp · .hwpx · .md 선택 · 드래그</span>
      </label>
      <span class="file-chip-hint">
        (.md 와 .md.assets 페어를 같이 골라도 됩니다)
      </span>
    `;
    rebindFileInput();
    return;
  }
  fileChipEl.classList.add("loaded");
  const tagClass = loaded.kind.split("+")[0]; // hwp / hwpx / md
  const tagLabel = loaded.kind.toUpperCase();
  const sizeKb = (loaded.size / 1024).toFixed(1);
  fileChipEl.innerHTML = `
    <label class="file-picker">
      <input type="file" id="file" accept=".hwp,.hwpx,.md,.markdown" multiple />
      <span class="file-picker-cta">재선택</span>
    </label>
    <span class="file-tag ${tagClass}">${tagLabel}</span>
    <span class="file-name" title="${escapeAttr(loaded.name)}">${escapeText(
    loaded.name,
  )}</span>
    <span class="file-meta">${sizeKb} KB</span>
    <button type="button" class="file-reset" id="file-reset">✕ 닫기</button>
  `;
  rebindFileInput();
  $<HTMLButtonElement>("file-reset").addEventListener("click", resetLoaded);
}

/// HTML escapers for the chip rendering.
function escapeText(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}
function escapeAttr(s: string): string {
  return escapeText(s).replace(/"/g, "&quot;");
}

/// Replacing the file-chip's innerHTML throws away the previous
/// `<input>` element along with its bound listeners. Re-grab the
/// new one and re-attach the change handler so the file picker
/// stays live.
function rebindFileInput(): void {
  const fresh = $<HTMLInputElement>("file");
  fresh.addEventListener("change", () => {
    const files = Array.from(fresh.files ?? []);
    void handleFiles(files);
  });
}

function resetLoaded(): void {
  if (ourIr !== null) {
    disposeDoc(ourIr);
    ourIr = null;
  }
  lastBuffer = null;
  lastFileName = null;
  lastBufferLoadedInEditor = false;
  markdownEl.textContent = "";
  copyBtn.disabled = true;
  mdDlBtn.disabled = true;
  hwpxDlBtn.disabled = true;
  pdfBtn.disabled = true;
  htmlBtn.disabled = true;
  mdPreviewEl.innerHTML =
    '<p class="hint">파일을 선택하면 여기에 HTML 미리보기가 나타납니다.</p>';
  renderFileChip(null);
  setStatus("ready");
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
    assetModeInt(),
    assetDpiInt(),
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
  markdownEl.hidden = target !== "markdown";
  if (target === "html") {
    renderStructuredHtml();
  } else if (target === "editor") {
    void ensureEditorLoaded();
  }
  // PDF / HTML download buttons live in the right-pane output card
  // now (no longer tab-bound). They're available whenever the IR
  // is loaded — the underlying `exportHtml` doesn't care which tab
  // is visible.
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
  // Inspect the input ZIP before handing it to wasm so the user
  // can compare the file we received against what `loadHwp` came
  // back with — discrepancies between this list and the IR's
  // `bin_data` summary point straight at the wasm-side reader.
  if (file.name.toLowerCase().endsWith(".hwpx")) {
    logHwpxEntries(`upload ${file.name}`, bytes);
  }

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
    logIr(`loadHwp(${file.name})`, ourIr);
    const m = renderMarkdown(ourIr);
    const ms = Math.round(performance.now() - started);
    setStatus(`loaded in ${ms}ms · MD ${m.bytes.toLocaleString()} bytes`);
    renderFileChip({
      kind: detectKind(file.name),
      name: file.name,
      size: bytes.length,
    });
  } else {
    ourIr = null;
    markdownEl.textContent = "";
    copyBtn.disabled = true;
    mdDlBtn.disabled = true;
    hwpxDlBtn.disabled = true;
    setStatus(`loadHwp 실패: ${String(irResult.reason)}`, true);
    renderFileChip(null);
  }
  pdfBtn.disabled = ourIr === null;
  htmlBtn.disabled = ourIr === null;

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
async function handleMarkdownFile(
  file: File,
  sourceOverride?: SourceKind,
): Promise<void> {
  const text = await file.text();
  // Reject `.assets.md` companion uploaded on its own — without the
  // matching body it carries only base64 footers and the importer
  // would fall through to the empty-body GFM path.
  if (sourceOverride !== "md+assets" && detectMdKind(text) === "assets") {
    setStatus(
      "이 파일은 split 모드의 .assets.md 동반 파일입니다. 본문 .md와 함께 같이 업로드하세요.",
      true,
    );
    return;
  }
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
  logIr(`importMarkdown(${file.name})`, ourIr);
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
  pdfBtn.disabled = false;
  htmlBtn.disabled = false;
  const ms = Math.round(performance.now() - started);
  setStatus(`loaded .md in ${ms}ms · ${text.length.toLocaleString()} chars`);
  renderFileChip({
    kind: sourceOverride ?? "md",
    name: file.name,
    size: text.length,
  });
}

function isMarkdownFile(name: string): boolean {
  const lower = name.toLowerCase();
  return lower.endsWith(".md") || lower.endsWith(".markdown");
}

/// Filename → SourceKind. `.hwpx` wins over `.hwp` because the
/// extension is more specific. `.md`/`.markdown` map to "md"; the
/// "md+assets" badge is only set when the paired-upload path
/// (`handleFiles`) explicitly overrides.
function detectKind(name: string): SourceKind {
  const lower = name.toLowerCase();
  if (lower.endsWith(".hwpx")) return "hwpx";
  if (lower.endsWith(".hwp")) return "hwp";
  return "md";
}

/// Strip the directory path and trailing extension from a filename —
/// `"folder/plan.hwpx"` → `"plan"`. Used as the stem for download
/// filenames (`plan.md`, `plan.pdf`).
function stemOf(name: string): string {
  const base = name.split(/[\\/]/).pop() ?? name;
  const dot = base.lastIndexOf(".");
  return dot > 0 ? base.slice(0, dot) : base;
}

// Initial chip render binds the file <input> change handler. After
// every renderFileChip(...), the input is fresh and rebound by
// rebindFileInput(). Drag-and-drop on the body still routes through
// the dispatcher.
renderFileChip(null);

document.body.addEventListener("dragover", (e) => e.preventDefault());
document.body.addEventListener("drop", (e) => {
  e.preventDefault();
  const files = Array.from(e.dataTransfer?.files ?? []);
  void handleFiles(files);
});

/// Pair-aware dispatcher. When the user picks (or drops) multiple
/// files, treat a `.md` body alongside an `.assets.md` companion as
/// a paired split-asset MD bundle — concatenate them and route
/// through `handleMarkdownFile`. Anything else falls back to the
/// single-file path.
async function handleFiles(files: File[]): Promise<void> {
  if (files.length === 0) return;
  if (files.length === 1) {
    void handleFile(files[0]);
    return;
  }
  // Content-based pairing: classify each candidate by its
  // format-dispatch stamp on the first non-blank line. macOS
  // Finder mangles `.assets.md` into `(N).md` when the base name
  // collides with the body file, so filename-suffix detection is
  // unreliable; the comment header is authoritative.
  const candidates = files.filter((f) => isMarkdownFile(f.name));
  const tagged = await Promise.all(
    candidates.map(async (f) => {
      const body = await f.text();
      return { file: f, body, kind: detectMdKind(body) };
    }),
  );
  const body = tagged.find((t) => t.kind === "llm" || t.kind === "human");
  const assets = tagged.find((t) => t.kind === "assets");
  if (body && assets) {
    const merged = `${body.body}\n\n${assets.body}`;
    const synthetic = new File([merged], body.file.name, {
      type: "text/markdown",
    });
    void handleMarkdownFile(synthetic, "md+assets");
    return;
  }
  // No recognised pair — fall through to first file.
  void handleFile(files[0]);
}

/// Read the format-dispatch comment on the first non-blank line.
function detectMdKind(text: string): "llm" | "human" | "assets" | "unknown" {
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (line.length === 0) continue;
    const m = line.match(/^<!-- hwp-transpiler: format=([a-z]+) -->$/);
    if (!m) return "unknown";
    if (m[1] === "llm") return "llm";
    if (m[1] === "human") return "human";
    if (m[1] === "assets") return "assets";
    return "unknown";
  }
  return "unknown";
}

for (const el of [
  llmModeEl,
  emitRolesEl,
  domainHintsEl,
  emitStylesEl,
  assetModeEl,
  assetDpiEl,
]) {
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
  // Split mode also yields a `<stem>.assets.md` companion. The
  // visible textarea shows main only; pull the companion direct
  // from wasm and drop it as a second download.
  if (ourIr !== null && assetModeInt() === 2) {
    try {
      const assets = exportMarkdownAssets(
        ourIr,
        llmModeEl.checked,
        emitRolesEl.checked,
        emitRolesEl.checked,
        domainHintsEl.checked,
        emitStylesEl.checked,
        assetDpiInt(),
      );
      if (assets.length > 0) {
        downloadBlob(
          new Blob([assets], { type: "text/markdown;charset=utf-8" }),
          `${currentStem}.assets.md`,
        );
      }
    } catch (err) {
      setStatus(`.assets.md 추출 실패: ${String(err)}`, true);
    }
  }
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
    logIr(`saveHwpx(input)`, ourIr);
    const bytes = saveHwpx(ourIr);
    const ms = Math.round(performance.now() - started);
    logHwpxEntries(`saveHwpx(output) ${currentStem}.hwpx`, bytes);
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
  if (!ourIr) return;
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
  if (!ourIr) return;
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

wireMobileBar();

// Share-relay entry point. When the page is opened with `?fetch=<url>`
// (typically by an Apple Shortcut that uploaded a HWP file to S3 via
// our Lambda signing endpoint), pull the bytes down and dispatch
// through the same `handleFiles` path as a drag-and-drop. The bytes
// never persist on our infra — the presigned URL is single-shot and
// S3 lifecycle deletes the object within 1 hour.
await consumeFetchParam();

async function consumeFetchParam(): Promise<void> {
  // Pull the raw substring after `?fetch=` instead of going through
  // URLSearchParams. An S3 presigned URL contains its own `&`-joined
  // signing params; URLSearchParams would clip the value at the
  // first `&`, leaving a useless URL that S3 rejects. Apple Shortcut
  // appends the presigned URL verbatim (no percent-encoding), so we
  // mirror that and read everything after `?fetch=` as the URL.
  const search = window.location.search;
  const idx = search.indexOf("?fetch=");
  // `?` may have been swapped to `&` by another wrapper — accept both.
  const altIdx = search.indexOf("&fetch=");
  const start = idx !== -1 ? idx + "?fetch=".length : altIdx !== -1 ? altIdx + "&fetch=".length : -1;
  if (start === -1) return;
  const fetchUrl = search.substring(start);
  if (!fetchUrl) return;

  // Strip the param immediately so a page refresh doesn't re-trigger
  // (and so the signed URL doesn't linger in the address bar).
  history.replaceState(null, "", window.location.pathname);

  setStatus("공유받은 파일 가져오는 중…");
  try {
    const resp = await fetch(fetchUrl);
    if (!resp.ok) {
      throw new Error(`HTTP ${resp.status}`);
    }
    const buf = new Uint8Array(await resp.arrayBuffer());

    // Recover filename from Content-Disposition (set by the signing
    // Lambda via ResponseContentDisposition). Falls back to a sane
    // default so the rest of the pipeline always has a name.
    const disp = resp.headers.get("content-disposition") || "";
    const m = disp.match(/filename="?([^"]+)"?/i);
    const name = m ? m[1] : "shared.hwp";

    const file = new File([buf], name, {
      type: resp.headers.get("content-type") || "application/octet-stream",
    });
    void handleFiles([file]);
  } catch (err) {
    setStatus(`공유 파일 로드 실패: ${err instanceof Error ? err.message : String(err)}`);
    console.warn(TAG, TAG_STYLE, "share-fetch failed:", err);
  }
}

// Mobile floating bar — wires the file picker to the existing
// handleFiles dispatcher and proxies the two visible buttons (HTML,
// MD) to the desktop buttons hidden inside the right-column cards.
// The desktop buttons remain the source of truth for `disabled`
// state; we mirror that onto the mobile twins via MutationObserver.
function wireMobileBar(): void {
  const fileInput = document.getElementById(
    "mobile-file",
  ) as HTMLInputElement | null;
  const mHtml = document.getElementById(
    "mobile-html-download",
  ) as HTMLButtonElement | null;
  const mMd = document.getElementById(
    "mobile-md-download",
  ) as HTMLButtonElement | null;
  if (!fileInput || !mHtml || !mMd) return;

  fileInput.addEventListener("change", () => {
    const files = Array.from(fileInput.files ?? []);
    if (files.length > 0) {
      void handleFiles(files);
    }
    // Reset so picking the same file again re-fires `change`.
    fileInput.value = "";
  });

  const mirror = (src: HTMLButtonElement, dst: HTMLButtonElement): void => {
    const sync = () => {
      dst.disabled = src.disabled;
    };
    sync();
    new MutationObserver(sync).observe(src, {
      attributes: true,
      attributeFilter: ["disabled"],
    });
    dst.addEventListener("click", () => {
      if (!src.disabled) src.click();
    });
  };

  mirror(htmlBtn, mHtml);
  mirror(mdDlBtn, mMd);
}
