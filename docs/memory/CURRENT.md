# 현재 위치

**지금**: Phase 2b (caption IR surface) 완료. `PictureControl.caption_text`
가 gso 체인 안의 LIST_HEADER + 자식 PARA_TEXT 로부터 추출되어
타입된 IR 로 올라옴. TRL fixture 9개 picture 중 6개 caption 실 추출.

**최근 ship**:
- `PictureControl` 에 `caption_text: Option<String>` 필드 추가.
- `body_text::parse_paragraph` 의 gso 상태머신 확장: CTRL_HEADER "gso "
  → LIST_HEADER (at child_lvl+1) 를 만나면 caption 수집 모드 진입,
  deeper PARA_TEXT 를 수집, SHAPE_COMPONENT 에서 종료. raw_records 는
  변경하지 않으므로 라운드트립 불변.
- `markdown::emit_picture` 가 `{{그림 N. <caption>}}` 로 emit.
  HWP 자동 번호 필드 (`"그림 ￼. ..."`) 의 FFFC 제거 후 생기는 `"그림 . "`
  prefix 를 `strip_caption_label_prefix` 로 제거 (그림/표/Figure/Table).
- Phase 2a-ii 완료: `hwp-to-md` CLI `--no-assets`, `--assets-dir=<path>`.
- Heading fix: multi-paragraph 박스 제목도 `##` 승격 (TRL 7장 이슈).

**알려진 갭** (다음 follow-up):
- **셀-임베드 picture 의 MD emission**: 현재 `emit_picture` 는 top-level
  paragraph 의 picture control 만 emit. TRL 에서는 9개 중 8개가 표
  안에 있어 MD 에 placeholder 가 나오지 않음 (IR 에는 존재). 캡션은
  저장되어 있으나 MD 표면으로 안 올라옴. 기존 "Cell-embedded pictures
  are silently dropped (Phase 2 follow-up)" 주석에 이미 기록된 별도
  작업. 해결하려면 `emit_table_as_list` / `emit_cell_line` 에 picture
  emit 분기 추가 필요.
- **Non-picture gso 의 caption**: line/rect/ole 같은 비-picture shape 은
  pending_picture 가 clear 되면서 caption 도 같이 버려짐. 이 도형
  type 들에 독립적인 IR control 타입이 생기면 그때 연결.

**다음 후보**:
- 위 follow-up (셀-임베드 picture MD emission) — Phase 2b 의 자연스러운
  완결
- L1 (LLM-friendly layer skeleton): `MdOptions.llm: Option<LlmOptions>`,
  section/table/cell/figure id 만 emit. 기존 human 출력 불변 (opt-in).

**막힌 것**: 없음.

**작업 트리**: 깨끗. 테스트: 186/186 green.

**빠른 컨텍스트**:
- 라운드트립이 1순위 목표; 마크다운 품질은 2순위. 명시적 결정은
  `docs/journal/2026-04-22-rowspan-marker-reverted.md` 의 "시각적 이득
  위해 마크다운에서 lossy 변환 금지" 항목.
- 스펙 사실은 이 디렉토리의 `hwp5-spec-notes.md` 에.
- hwplib 포팅 맵은 이 디렉토리의 `hwplib-mapping.md` 에.
- 저널 엔트리 (`docs/journal/`) 는 *언제·왜* 를 기록; 여기 문서는
  *지금 무엇이 사실인가* 를 기록.

stale해지면 (HEAD가 이동하거나 테스트가 변하거나 막힌 것 생기면)
업데이트할 것 — 그게 이 문서의 존재 이유.
