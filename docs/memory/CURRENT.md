# Where we are

**Now**: Phase 2a-i complete; Phase 2a-ii pending.

**Last shipped**: PictureControl wiring — body_text parse_paragraph
extracts `bin_id` + `width_hwpu` + `height_hwpu` from the
`CTRL_HEADER "gso " → SHAPE_COMPONENT → SHAPE_COMPONENT_PICTURE` chain.
TRL fixture surfaces 9 `ControlKind::Picture` controls (one per image).

**Next**: Phase 2a-ii — `hwp-to-md` writes `<doc>.assets/BIN<N>.<ext>`
sidecar files + markdown emits `![](./<doc>.assets/BIN<N>.<ext>)
{width=Xmm; height=Ymm}` followed by `{{그림 N.}}` placeholder.
Cross-reference `PictureControl.bin_id` against `DocInfo.bin_data` to
get the extension, then look up the actual stream bytes in
`IrDocument.bin_data`.

**After**: Phase 2b — caption text extraction (caption is a
sub-paragraph inside the gso control). The placeholder body becomes
`{{그림 N. <caption text>}}`.

**Blockers**: none.

**Working tree**: clean. Tests: 155/155 green.

**Quick context**:
- Round-trip is the primary goal; markdown quality is secondary. See
  `docs/journal/2026-04-22-rowspan-marker-reverted.md` for the explicit
  "no lossy markdown for visual gain" decision.
- Spec findings live in `hwp5-spec-notes.md` (this directory).
- hwplib porting map lives in `hwplib-mapping.md` (this directory).
- Journal entries (`docs/journal/`) capture *why* and *when*; the docs
  here capture *what is true now*.

When this is stale (HEAD moves, tests change, blockers appear),
update it — that's the point.
