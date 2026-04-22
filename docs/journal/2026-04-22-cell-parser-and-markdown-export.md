# 2026-04-22 — cell LIST_HEADER fix + Markdown export quality round

**Context**: First end-to-end run of `hwp-to-md` on the TRL R&D
form fixture (`260420-1. 연구개발계획서…fin.hwp`, 5 MB, 1648 paragraphs,
53 tables incl. nested) surfaced two large-scale defects that no unit
test had caught: every table cell coordinate was garbage
(`[65024,11] span 36097×65025`), and once that was fixed every
real-world table either dumped to a flat bullet list or rendered as a
single 6 KB-wide row. Both classes of bug were addressed in a series
of commits this session.

## Verified — hwplib `ListHeaderForCell` byte layout

Our `streams/list_header.rs::parse_cell` had assumed:

  - a `u16` `paraCount` preamble (so an ~21-byte head before the cell
    suffix), and
  - a fixed 26-byte cell suffix consisting of col/row/span × 4
    `u16` + width/height `u32` + 4 × padding `u16` + borderFillId.

Reading the canonical hwplib source
(`reader/.../tbl/ForCell.java::listHeader`) showed the real layout is:

```
sInt4  paraCount        (4)
uInt4  property         (4)
uInt2  colIndex / rowIndex / colSpan / rowSpan          (4 × 2 = 8)
uInt4  width / height                                   (2 × 4 = 8)
uInt2  leftMargin / rightMargin / topMargin / bottomMargin  (8)
uInt2  borderFillId     (2)
uInt4  textWidth        (4)
                                              (38 bytes fixed)
[opt]  uInt1 fieldNameFlag (0xff → ParameterSet) + 8-byte zero pad
```

— `paraCount` is `sInt4`, not `u16`; preamble is **8 bytes**, not 21.
The fixed region is **38 bytes** (we were missing `textWidth`). And the
trailing `flag + ParameterSet + 8-byte zero pad` (always emitted by
hwplib's writer) makes the previous "read from the end of the record"
strategy unsafe — the suffix landed inside the trailer, producing the
~1-byte-shifted garbage we observed.

The fix re-reads from offset 0 with the canonical layout.

**Consequence**: The same offset-from-end trap likely exists in our
yet-unwritten readers for the other LIST_HEADER variants (footnote,
endnote, header, footer, text-box). When wiring those, port the
canonical writer/reader pair from hwplib and parse from the start.

## Verified — Hancom PUA bullet ranges

한컴 fonts encode `①..⑳` (and likely several other enumeration glyphs)
as PUA codepoints. We expected only the BMP range `U+F2B1+`, but the
TRL fixture's `① 과제 개요` actually arrived as `U+F02B1` —
**Supplementary PUA-A** (U+F0000+). Both ranges encode the same glyphs
at the same offsets:

  - `U+F02B1..U+F02C4` → `U+2460..U+2473` (`①..⑳`)
  - `U+F2B1..U+F2C4`   → same

The supplementary range is what newer hwp.exe emits; the BMP form is
in older docs. Both should be normalised in `clean_text` so the result
survives outside HCR Dotum / Batang.

**Open question**: There are likely Hancom PUA codepoints for other
enumeration styles (`㉠..㉭`, `㊀..㊉`, parenthesised `⑴..⒇`, etc.).
Discovery is fixture-driven — when one shows up as tofu, look at its
codepoint, find the corresponding standard glyph, extend the
translator. No comprehensive table is publicly documented.

## Decision — table classification heuristic for Markdown export

Final dispatch order in `emit_table`, all conditions checked top-down:

  1. `try_unwrap_wrapper_table` — 1×1 with no body text + exactly one
     nested Table. Strip the wrapper; recurse into the inner at the
     same depth.
  2. `try_table_as_heading` — exactly one non-empty cell, single short
     line ≤ 80 chars, prefix matches `<digits>. ` (→ `##`) or
     `(...) ` (→ `###`).
  3. `try_table_as_passage` (top-level only) — single row, exactly one
     non-empty cell, ≤ 100 chars after joining paragraphs with spaces,
     no controls. Emit as plain prose.
  4. `try_build_md_grid` — every `row_span == 1`; cells (after
     expanding `col_span` with empty siblings) tile the grid without
     overlaps or holes; no nested tables. Emit as MD grid.
  5. Otherwise → `emit_table_as_list`. In the bullet path:
     - runs of unspanned empty cells in the same row collapse into
       `[r,c1..c2]: (empty)`;
     - cell text inlines with ` · ` if joined length ≤ 200 chars,
       else explodes into nested `  - …` sub-bullets per paragraph.

Three numeric thresholds (80 / 100 / 200 chars) were tuned against
this single fixture only.

**Why this shape**: Conservative-then-permissive. Each early-exit
(unwrap / heading / passage) discards the table abstraction entirely,
so they have the strictest predicates. The grid path preserves
table-ness when GFM can express it (no rowspan, no holes). The bullet
path handles everything else without losing data — its empty-range
collapse and length-aware inline/explode pair existed in earlier forms
that either drowned in noise (every empty cell on its own line) or
flattened narrative copy (the §2 box of this fixture used to render
as one ~6 KB-wide ` · `-joined line).

**Consequence**: The thresholds are corpus-of-one. The next fixture we
add (especially anything academic — KSII, Springer Korean templates)
should re-evaluate.

## Open — row_span > 1 forces bullet path; lossy-but-readable
alternative not yet explored

GFM has no rowspan, so any vertically merged cell currently routes
through the bullet path even when the rest of the table would render
cleanly as a grid. On the TRL fixture the §1 12×6 form table is the
clearest victim: only one cell (`[8,0] span 2×1: 개발 방법`) has
`row_span=2`, but that's enough to push the entire 12×6 onto the
bullet path.

Two viable approaches:

  - Expand row_span by repeating the merged text in every covered row
    (lossy: visually duplicates the label, but the table renders).
  - Expand row_span by emitting the text once and filling the lower
    rows with ` ↑ ` or empty (lossy: viewer can't tell whether the
    blank means merged or genuinely empty).

Decision deferred until we see whether the bullet rendering of large
form tables is actually a problem in user-facing previews — for raw
Markdown export it may be acceptable.

## Open — single-fixture corpus

Round-trip and quality verification ride on one HWP file. Adding a
second fixture with a different style (free-form long-form, heavy
images, equation-heavy academic, password-protected) would be the
fastest way to find the next class of bug. The journal's earlier
`fixture_tables_populate_from_trl` is the template.
