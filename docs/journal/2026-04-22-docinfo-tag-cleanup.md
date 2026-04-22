# 2026-04-22 — DocInfo tag identification (Round 3 minimum)

**Context**: The end of yesterday's `2026-04-22-rounds-1-2.md` flagged
two DocInfo records that arrived in `raw_records` with tags we hadn't
enumerated: `0x0020` and `0x005E`. The session-3 plan needed an answer
before deciding which records to type next.

**Finding** (from hwplib `object/etc/HWPTag.java`):

  - `0x0020` = **TRACK_CHANGE_INFO** (변경 추적 정보) — track-changes
    metadata block.
  - `0x005E` = **FORBIDDEN_CHAR** (금칙처리 문자) — Korean line-break
    forbidden-character configuration.

Both are valid HWP5 spec tags, just niche. Neither has a published
binary layout that I could find — hwplib has stub readers but the
field semantics aren't in the public spec.

While inspecting `HWPTag.java` I also noted three further DocInfo tags
we hadn't enumerated: `0x005C` MEMO_SHAPE, `0x0060` TRACK_CHANGE
(body), `0x0061` TRACK_CHANGE_AUTHOR. None has appeared in our two
fixtures yet, but they're part of the spec.

**Decision**: Add all five tag constants to `streams::doc_info::tag`
so future code can reference them by name rather than as bare hex.
**Do not** wire typed parsers — `raw_records` already preserves the
bytes verbatim, and the strangler-fig writer falls back to
`stream_bytes` when nothing has been mutated, so round-trip stays
correct.

**Why no parsers**: The five tags split into two camps. Track-changes
records (0x0020 / 0x0060 / 0x0061) carry editorial state that a
transpiler doesn't need to interpret to faithfully round-trip — and
mucking with them risks breaking 한컴's signing/integrity checks on
distributed docs. Forbidden-char and memo-shape are similarly niche.
Verbatim passthrough is a safer default than a guessed layout.

**Consequence**: When we eventually add `TRACK_CHANGE_*` parsers, they
need a fixture that actually contains track changes — the TRL form
doesn't exercise it.

**Open**: BinData (`0x0012`) is still the highest-value untyped
DocInfo record — it carries image references that the Markdown
exporter currently can't surface as `![](...)`. Wiring requires (a) a
typed parser for the BinData record body, (b) moving `/BinData/*`
streams out of `unknown_streams` into a typed `binary_files` map, and
(c) cross-referencing from `PictureControl` in body. Estimated as the
next session-sized chunk.
