# hwplib → ts-hwp-transpiler 포팅 맵

hwplib 가 HWP5 binary read/write의 canonical 레퍼런스 (neolord0/hwplib,
Java, Apache 2.0). 새 record를 포팅할 때 hwplib 클래스가 bit-for-bit
비교 대상. 이 문서는 어떤 Java 클래스를 어디로 포팅했는지 추적.

새 포팅이 land하면 행 추가. hwplib upstream이 변경되면 이게 diff target.

---

## DocInfo

| hwplib (Java)                                    | ts-hwp-transpiler (Rust) |
|--------------------------------------------------|--------------------------|
| `object/etc/HWPTag`                              | `streams/doc_info::tag` (DocInfo 절반) + `streams/body_text::tag` (BodyText 절반) |
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
| `reader/.../paragraph/control/ForCtrlHeader`     | `streams/ctrl_header` (code만; table 외 body는 Unknown) |
| `reader/.../paragraph/control/tbl/ForTable`      | `streams/table` |
| `reader/.../paragraph/control/tbl/ForCell`       | `streams/list_header::parse_cell` |
| `reader/.../paragraph/control/gso/part/ForCtrlHeaderGso` | `streams/gso_picture::parse_gso_size` (width/height만) |
| `reader/.../paragraph/control/gso/part/ForShapeComponent` | `streams/gso_picture::parse_shape_component_id` (gsoId만) |
| `reader/.../paragraph/control/gso/ForControlPicture` | `streams/gso_picture::parse_picture_bin_id` (binItemID만) |
| `reader/.../paragraph/control/gso/ForGsoControl` (state machine) | `streams/body_text::parse_paragraph` 의 pending_picture 분기 |
| `reader/.../paragraph/control/gso/Other Controls`| — passthrough (line/rect/ellipse/arc/polygon/curve/OLE/textart/container) |

## Writers

대칭 — reader가 typed encoder를 갖춘 경우 writer pair는 같은 모듈에
공존 (`emit()` 가 `parse()` 옆). 아래 표에 안 나오는 writer는 reader도
없는 경우.

| hwplib (Java)                                    | ts-hwp-transpiler (Rust) |
|--------------------------------------------------|--------------------------|
| `writer/HWPWriter` + `ForFileHeader`             | `hwp::HwpWriter`, `streams/file_header::emit` |
| `writer/docinfo/ForDocInfo`                      | `hwp::writer::encode_doc_info` (현재 `stream_bytes` verbatim 폴백 — 노트 참고) |
| `writer/bodytext/ForBodyText`                    | `hwp::writer::encode_section` (verbatim only — typed 재인코딩은 미해결) |

## 포팅 스타일 노트

- field-by-field로 번역, hwplib offset 그대로 유지. hwplib가 `sInt4`
  를 읽으면 우리는 `i32::from_le_bytes` 사용.
- hwplib가 Java enum (`BinDataType.Link`)을 쓰는 경우 우리는 IR struct
  의 `pub const` discriminant 사용. 라운드트립 관련 비트는 raw 형태로
  유지 (`property: u16`) + helper method 제공.
- 각 포팅된 모듈의 테스트는 `roundtrip()` helper로 emit→parse 동등성
  확인. 패턴은 `streams/bin_data::tests` 참고.
- hwplib reader가 variant-dependent 레이아웃을 갖는 경우 (cell
  list-header trailer, BinData type-dependent 필드), 우리 파서는
  property 워드에서 variant를 *re-derive* 해야 하지 고정 길이 가정
  금지. 셀 파서가 이 점에서 한 세션 비용 — `2026-04-22-cell-parser-
  and-markdown-export.md` 참고.

## Vendor된 sample fixture

`crates/codec/tests/fixtures/` 가 hwplib `sample_hwp/` 의 `blank.hwp`
와 `merging-cell.hwp` 를 vendor. re-fetch 명령은 해당 디렉토리 README.
