# 현재 위치

**지금**: L2 (cell role 분류기) LLM 출력에 연결됨. `--llm --emit-roles`
시 각 CELL 에 `role=header|label|value|spacer` 가 붙음. 기존 visual
classifier (`semantics::visual::classify_roles`) 를 DocInfo/TableCell
adapter 로 감싸서 연결.

**최근 ship**:
- `Fill::back_color() -> Option<(r,g,b,a)>` 추가. HWP5 ColorFill
  body byte[3] 가 실제 alpha 가 아니라 reserved/flag byte (TRL 에서
  E5E5E5 같은 불투명 회색이 byte[3]=0 으로 저장됨) 이므로 KIND_COLOR
  fill 은 무조건 opaque `a=0xFF` 로 반환. Transparency 는 KIND 플래그
  로만 판정.
- `core::semantics::visual_adapter`: `DocInfoResolver`(BorderFillResolver
  구현) + `VisualExtract for TableCell`. BorderFill id 는 1-indexed
  (hwplib 관례), id=0 은 "no style" → None.
- `markdown_llm::emit_table` 이 `emit_roles` 시 `classify_roles()` 를
  한 번에 테이블 단위로 호출, 결과를 cell 별로 emit.
- CLI `--emit-roles`, `--emit-editable` flag. `--llm` 없이 쓰면 오류.
- TRL fixture 실측: 1062 cells → 115 header / 40 label / 907 value.
  Unknown 없음.

**알려진 갭**:
- **classifier 가 Yellow-centric**. TRL 은 연파란회색(#DFE6F7)
  기반이라 label 판정이 보수적 (40/1062). Classifier 확장은 별도
  tuning pass — "Cyan accent + first_col + short text" 같은 규칙
  추가 필요. 하지만 안전 방향 (보수적 label, 나머지는 value) 이므로
  즉시 위험 없음.
- **editable 분류기 미구현** (L3 과제). `--emit-editable` 은 현재
  `editable=unknown` placeholder 만 emit.
- **Non-picture gso 의 caption** (Phase 2b 원래 갭).

**다음 후보**:
- L3: editable 추정 (role=value + 단일 paragraph + 수식/숫자 아님).
- L4: figure/caption 전역 domain hint (performance_metrics, budget).
- Classifier tuning: non-Yellow label colour 대응.
- Preview layer (render crate) — IR → HTML with rowspan/colspan.

**막힌 것**: 없음.

**작업 트리**: 깨끗. 테스트: 207/207 green.

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
