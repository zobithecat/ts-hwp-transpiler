# 현재 위치

**지금**: L3 (editable 추정) LLM 출력에 연결됨. `--llm --emit-editable`
시 각 CELL 에 `editable=true|false|unknown` 추가. 보수적 규칙
(role=value + 단일 paragraph + no controls + 숫자 아님).

**최근 ship**:
- `markdown_llm::infer_editable` 추가. 규칙:
  1. role=Header/Label/Spacer → `false`
  2. paragraphs > 1 → `false`
  3. cell 안에 control (Table/Picture/Equation) → `false`
  4. 비어 있음 → `true` (fill-in slot)
  5. 텍스트의 ≥90% 가 숫자/구두점/공백 → `false` (계산값·날짜)
  6. 그 외 value → `true`
  7. role 없음 (classifier 비활성) → `unknown`
- `emit_editable` 활성시 자동으로 role 계산 (emit_roles 안 켜져도 내부
  계산). 사용자에게는 `role=` 안 보이지만 editable 판정은 유효.
- L2 (cell role 분류기): TRL 1062 cells → 115 header / 40 label /
  907 value.
- L3 (editable): TRL → 546 true / 516 false (header+label 155 +
  value-but-numeric/multi/control 361).

**알려진 갭**:
- **classifier tuning**: TRL #DFE6F7 연파란회색이 Label 로 안 잡힘.
  Yellow-tuned. 후속 tuning pass 에서 확장.
- **Non-picture gso 의 caption** (Phase 2b 갭).
- **`is_mostly_numeric` 임계치 90% 가 너무 빡빡/관대할 수 있음**.
  튜닝은 실제 사용 feedback 으로.

**다음 후보**:
- **Classifier tuning**: Cyan/Pale accent 도 label 로 승격.
- **L4**: figure/caption 전역 domain hint.
- **Preview layer** (render crate) — IR → HTML with rowspan/colspan.
- **Non-picture gso caption** — line/rect/ole shape 의 caption 살리기.

**막힌 것**: 없음.

**작업 트리**: 깨끗. 테스트: 214/214 green.

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
