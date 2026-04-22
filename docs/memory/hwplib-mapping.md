# hwplib → ts-hwp-transpiler porting map

hwplib is our canonical reference for HWP5 binary read/write
(neolord0/hwplib, Java, Apache 2.0). When porting a new record, the
hwplib class is what we compare against bit-for-bit. This file tracks
which Java classes we've ported and where.

When a new port lands, add the row. When hwplib changes upstream and
we need to follow, this is the diff target.

---

## DocInfo

| hwplib (Java)                                    | ts-hwp-transpiler (Rust) |
|--------------------------------------------------|--------------------------|
| `object/etc/HWPTag`                              | `streams/doc_info::tag` (DocInfo half) and `streams/body_text::tag` (BodyText half) |
| `reader/ForFileHeader`                           | `streams/file_header` |
| `reader/docinfo/ForDocInfo`                      | `reader::populate_typed_views` (dispatcher) |
| `reader/docinfo/ForDocumentProperties`           | `streams/document_properties` |
| `reader/docinfo/ForIDMappings`                   | `streams/id_mappings` |
| `reader/docinfo/ForFaceName`                     | `streams/face_name` |
| `reader/docinfo/ForBorderFill`                   | `streams/border_fill` |
| `reader/docinfo/ForCharShape`                    | `streams/char_shape` |
| `reader/docinfo/ForParaShape`                    | `streams/para_shape` |
| `reader/docinfo/ForStyle`                        | `streams/style` |
| `reader/docinfo/ForBinData`                      | `streams/bin_data` |
| `reader/docinfo/ForTabDef`                       | — passthrough only |
| `reader/docinfo/ForNumbering`                    | — passthrough only |
| `reader/docinfo/ForBullet`                       | — passthrough only |
| `reader/docinfo/ForDocData`                      | — passthrough only |
| `reader/docinfo/ForDistributeDocData`            | — passthrough only |
| `reader/docinfo/ForCompatibleDocument`           | — passthrough only |
| `reader/docinfo/ForLayoutCompatibility`          | — passthrough only |

## BodyText

| hwplib (Java)                                    | ts-hwp-transpiler (Rust) |
|--------------------------------------------------|--------------------------|
| `reader/bodytext/ForBodyText`                    | `streams/body_text::parse_section` (dispatcher) |
| `reader/bodytext/paragraph/ForParagraph`         | `streams/body_text::parse_paragraph` |
| `reader/.../paragraph/ForParaHeader`             | `streams/paragraph_header` |
| `reader/.../paragraph/ForNormalText`             | `streams/paragraph_text` |
| `reader/.../paragraph/ForCharShape`              | `streams/paragraph_char_shape` |
| `reader/.../paragraph/ForLineSeg`                | `streams/paragraph_line_seg` |
| `reader/.../paragraph/control/ForCtrlHeader`     | `streams/ctrl_header` (code only; body Unknown for non-table) |
| `reader/.../paragraph/control/tbl/ForTable`      | `streams/table` |
| `reader/.../paragraph/control/tbl/ForCell`       | `streams/list_header::parse_cell` |
| `reader/.../paragraph/control/gso/ForGsoControl` | **Phase 2a-i — TBD** |
| `reader/.../paragraph/control/gso/ForControlPicture` | **Phase 2a-i — TBD** |
| `reader/.../paragraph/control/gso/Other Controls`| — passthrough (line/rect/ellipse/arc/polygon/curve/OLE/textart/container) |

## Writers

Symmetric — when a reader has a typed encoder, the writer pair lives
next to it (`emit()` alongside `parse()` in the same module). Writers
not listed below are missing because their reader is also missing.

| hwplib (Java)                                    | ts-hwp-transpiler (Rust) |
|--------------------------------------------------|--------------------------|
| `writer/HWPWriter` + `ForFileHeader`             | `hwp::HwpWriter`, `streams/file_header::emit` |
| `writer/docinfo/ForDocInfo`                      | `hwp::writer::encode_doc_info` (currently falls back to `stream_bytes` verbatim — see notes) |
| `writer/bodytext/ForBodyText`                    | `hwp::writer::encode_section` (verbatim only — typed re-encode is open) |

## Notes on porting style

- We translate field-by-field, keeping hwplib's offsets exactly. When
  hwplib reads `sInt4` we use `i32::from_le_bytes`, etc.
- Where hwplib uses Java enums (`BinDataType.Link`), we use
  `pub const` discriminants on the IR struct. Round-trip-relevant
  bits stay in raw form (`property: u16`) with helper methods.
- Tests for each ported module use a `roundtrip()` helper that
  emit→parse and asserts equality. See `streams/bin_data::tests` for
  the pattern.
- When hwplib's reader has variant-dependent layout (cell list-header
  trailer, BinData type-dependent fields), our parser **must** re-derive
  the variant from the property word, not assume a fixed length. Cell
  parser regression on this point cost us a session — see
  `2026-04-22-cell-parser-and-markdown-export.md`.

## Sample fixtures vendored

`crates/codec/tests/fixtures/` carries `blank.hwp` and
`merging-cell.hwp` from hwplib's `sample_hwp/`. Re-fetch instructions
in that directory's README.
