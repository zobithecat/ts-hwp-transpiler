# 현재 위치

**지금**: Phase 2a-i 완료; Phase 2a-ii 진행 중 (마크다운 측 emit_picture
완료, CLI 사이드카 dump는 미완성).

**최근 ship**: PictureControl 와이어링 — body_text parse_paragraph가
`CTRL_HEADER "gso " → SHAPE_COMPONENT → SHAPE_COMPONENT_PICTURE` 체인
에서 `bin_id` + `width_hwpu` + `height_hwpu` 추출. TRL fixture에서
9개 `ControlKind::Picture` 컨트롤 surface (이미지 1:1 매칭).

이후 `markdown::to_markdown_with` + `MdOptions { assets_path }` 추가:
`assets_path` 가 주어지면 각 top-level picture가
`![](<prefix>/BIN<id>.<ext>){width=Xmm; height=Ymm}` + `{{그림 N.}}`
형태로 emit. assets_path 없으면 placeholder만 emit.

**다음**: Phase 2a-ii 완성 — `hwp-to-md` CLI가
- `<doc>.assets/` 디렉토리 생성
- `IrDocument.bin_data` 의 모든 BinaryEntry → `<assets>/<id>` 파일 dump
- `MdOptions { assets_path: Some(...) }` 로 `to_markdown_with` 호출
- `--no-assets` 옵션으로 끄기 가능

**이후**: Phase 2b — 캡션 텍스트 추출 (캡션은 gso 컨트롤 안의
sub-paragraph). placeholder가 `{{그림 N. <caption text>}}` 가 됨.

**막힌 것**: 없음.

**작업 트리**: 깨끗. 테스트: 155/155 green.

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
