# Where we are

**Now**: Phase 2a-i — body_text PictureControl wiring, in progress.

**Last shipped** (HEAD = `7cbed20`): BinData Phase 1.5 — `/BinData/<id>`
extension → `image/*` mime resolved at read time.

**Next**: Parse `CTRL_HEADER "gso "` + `SHAPE_COMPONENT_PICTURE` records
in `body_text::parse_paragraph` so `PictureControl.bin_id`,
`width_hwpu`, `height_hwpu` come out non-default. hwplib references:
`reader/bodytext/paragraph/control/gso/{ForGsoControl,ForControlPicture}.java`.

**After**: Phase 2a-ii — `hwp-to-md` writes `<doc>.assets/BIN<N>.png`
sidecar + markdown emits `![](./<doc>.assets/...){width=Xmm; height=Ymm}`
+ `{{그림 N.}}` placeholder. Caption text is Phase 2b.

**Blockers**: none.

**Working tree**: clean. Tests: 149/149 green.

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
