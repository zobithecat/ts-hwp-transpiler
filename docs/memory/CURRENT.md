# 현재 위치

**지금**: Phase 2b 완결 (caption IR + MD surface). 셀-임베드 picture
emission 까지 붙어 TRL fixture 9/9 picture 가 MD 에서 번호·캡션과
함께 표시됨. 다음은 L1 (LLM-friendly layer skeleton).

**최근 ship**:
- **Cell-embedded picture MD emission**: `emit_table_as_list` →
  `emit_cell_line` 이 `ControlKind::Picture` 도 iterate 하여
  bullet sub-item (`- ![](…)` + `- {{그림 N. caption}}`) 으로 emit.
  `picture_counter` 를 table pipeline 전체에 mut ref 로 threading.
  `try_build_md_grid` 는 셀에 picture 가 있으면 reject → bullet
  fallback 강제 (nested table 과 동일 정책).
- **Phase 2b caption IR**: `PictureControl.caption_text` 를 gso
  체인의 LIST_HEADER + 자식 PARA_TEXT 에서 추출. HWP 자동 번호
  필드 `"그림 ￼. ..."` 의 FFFC 제거 후 `"그림 . "` prefix 를
  strip_caption_label_prefix 로 제거.
- **Phase 2a-ii**: CLI `--no-assets`, `--assets-dir=<path>`, `<stem>.assets/`
  sidecar dump.
- **Heading fix**: multi-paragraph 박스 제목도 `##` 승격 (TRL 7장).

**알려진 갭**:
- **Non-picture gso 의 caption**: line/rect/ole 같은 비-picture shape 은
  pending_picture 가 clear 되면서 caption 도 같이 버려짐. 해당 도형
  타입들에 독립적인 IR control 타입이 생기면 그때 연결.
- **Caption 라벨 외국어 prefix**: `strip_caption_label_prefix` 는 현재
  "그림/표/Figure/Table" 네 가지만 처리. 다른 언어 확장 시 여기서.

**다음 후보**:
- **L1 (LLM-friendly layer skeleton)**: `MdOptions.llm: Option<LlmOptions>`
  추가. section/table/cell/figure id 만 emit, role/editable 은 전부
  `unknown` 으로 시작. 기존 human 출력 불변 (opt-in). stable id 설계는
  HWP5 native 식별자 (`ParagraphHeader.instance_id`, `BinData.bin_data_id`,
  TableCell.row/col) 에서 결정적 파생.

**막힌 것**: 없음.

**작업 트리**: 깨끗. 테스트: 189/189 green.

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
