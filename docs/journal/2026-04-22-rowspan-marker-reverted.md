# 2026-04-22 — directional row/col span markers tried, reverted

**Context**: The previous journal flagged "GFM has no rowspan, so any
`row_span > 1` forces the bullet path" as an Open question with two
candidate lossy expansions. I built one of them — anchor cell carries
text, continuation cells get directional markers (`↑` / `←` / `↖`
pointing back at the anchor) — and ran it on the TRL fixture
(commit `7c6f81a`, since reset).

The output looked great visually: the §1 12×6 project-overview table
folded into a single readable grid instead of ~70 bullet lines. But
when I checked the choice against the project's actual goal it didn't
hold up.

**Decision**: Reverted `7c6f81a` via `git reset --hard`. Markdown
output keeps the bullet path's `- [r,c] span N×M:` coordinate
annotation for any merged cell.

**Why**:

  - **Round-trip breaks.** A continuation cell holding `↑` is lossy:
    `md → hwp` doesn't know how to re-fuse the marked cells back into
    the original merge. The transpiler's primary purpose is `hwp ↔
    md`, not "produce nice-looking md".
  - **LLM readability is *better* with the annotation.** `[8,0]
    span 2×1: 개발 방법` makes the merge structure machine-explicit;
    `↑` makes the consumer infer it from glyph adjacency. The user's
    earlier "merged tables are hard to read" complaint was a *human*
    grievance, not an LLM one.
  - **Visual rendering is a different layer.** A future
    `crates/render` (or the wasm preview) can re-fuse spans into HTML
    `<table>` at render time. Doing the fusion in markdown source
    sacrifices source fidelity for a downstream layer's job.

**Consequence**: Three goals separate cleanly:

  - **Markdown source** — lossless, LLM-friendly, coordinate-explicit.
  - **Preview render** — human-friendly, may use HTML tables, never
    written back to disk.
  - **Round-trip** — guaranteed, because md never lost the span info.

`task.md`'s "No `<table>` tags" rule applies only to markdown export,
not to preview rendering.

**Open**: Preview/renderer crate hasn't been built yet — when it
lands, it should re-fuse `span N×M` annotations into HTML tables (or
canvas drawings) so the human-friendly view materialises there. That
work is outside the codec.
