# ts-hwp-transpiler

Bidirectional **HWP/HWPX ↔ Markdown** transpiler with browser-native
preview. Goal: round-trip Korean office documents through Markdown
without losing structure, and render the result either as plain MD
(for LLMs and editors) or as a high-fidelity preview (for humans).

> Status: early — reader covers the read path well, writer rounds
> through verbatim caches, Markdown export is usable for the test
> corpus. WASM and preview layers are stubbed. See
> `docs/memory/CURRENT.md` for the live state pointer.

## Layout

```
crates/
  core/          neutral IrDocument + Reader/Writer traits
  codec/         HWP5 reader/writer + Markdown exporter (this is most of the code)
  render/        backend-agnostic RenderCommand skeleton (canvas/SVG-ready)
  wasm/          wasm-bindgen surface for the browser preview
docs/
  memory/        live reference docs (current state, spec notes, hwplib porting map)
  journal/       append-only design log (decisions, why, what was reverted)
test/            personal fixtures (gitignored except .gitkeep)
```

## Quick start

```sh
# Build + test the whole workspace
cargo test --workspace

# HWP → Markdown
cargo run -p hwp-transpiler-codec --bin hwp-to-md -- path/to/input.hwp
# writes to ./path/to/input.md by default; pass `-` for stdout
```

A small fixture corpus is vendored from neolord0/hwplib under
`crates/codec/tests/fixtures/` (Apache 2.0). The round-trip suite is
gated on those + any personal HWPs you drop into `/test/`.

## What works today

- **/FileHeader, /DocInfo, /BodyText/Section{N}** — typed read +
  byte-equal round-trip via verbatim stream cache (mutated records
  trigger re-encode).
- **DocInfo records** typed: `DocumentProperties`, `IdMappings`,
  `FaceName ×7 slots`, `BorderFill`, `CharShape`, `ParaShape`,
  `Style`, `BinData`. Untyped tags pass through `raw_records`
  unchanged.
- **BodyText records** typed: paragraph header / text / char-shape
  runs / line segments, `LIST_HEADER` (cell), `TABLE` with cells, and
  the `gso → SHAPE_COMPONENT → SHAPE_COMPONENT_PICTURE` chain into
  `ControlKind::Picture`.
- **Embedded binaries** — `/BinData/<id>.<ext>` streams flow into
  `IrDocument.bin_data` with auto-resolved MIME.
- **Markdown export** with several quality passes: heading detection,
  colspan-only grid expansion, single-row decorative tables collapse
  to passages, decorative wrapper tables unwrap, long bullet cells
  explode into sub-bullets, empty cell ranges collapse, Hancom PUA
  circled-digit bullets normalise to standard `①..⑳`. Preserves
  merged-cell info as `[r,c] span N×M:` annotations (lossless for
  round-trip; the preview layer's job to render visually).

On the TRL R&D form fixture (5 MB, 53 tables, 9 embedded images): full
byte-equal round-trip + readable Markdown export.

## What's missing

- **Markdown writer** (md → hwp) is the next big arc. Today only the
  read direction is end-to-end; the writer surface uses verbatim
  caches.
- **WASM browser preview** — `crates/wasm` and `crates/render` are
  scaffolded but don't have a renderer.
- **Image markdown emission** — pictures parse into IR but the CLI
  doesn't yet dump sidecar files (`<doc>.assets/`); `MdOptions
  .assets_path` exists, the wiring is in progress.
- **Captions, equations, footnotes, track changes, etc.** are
  preserved verbatim but not surfaced.

## Documentation

- **`docs/memory/CURRENT.md`** — what's true *right now*. Read this
  first when picking up.
- **`docs/memory/hwp5-spec-notes.md`** — consolidated binary-format
  facts (HWP5 spec is incomplete in many areas, this is what we've
  verified).
- **`docs/memory/hwplib-mapping.md`** — Java
  [hwplib](https://github.com/neolord0/hwplib) → Rust file map.
- **`docs/journal/`** — design decisions, rejected alternatives,
  reverted attempts. The README of that directory describes the
  entry templates.
- **`task.md`** — original mission spec.

## License

(unset — TBD)
