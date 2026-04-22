# 현재 위치

**지금**: L4 (table domain hint) 추가. `--llm --emit-domain-hints` 시
`TABLE[...,kind=budget|institution_info|schedule|performance_metrics|
personnel]` 형태로 content-type hint 가 붙음. Unknown 은 elide.

**최근 ship**:
- `core::semantics::domain::{TableDomain, infer_table_domain}` 추가.
  Keyword 기반 scoring: Institution / Budget / Schedule / Performance
  / Personnel 다섯 카테고리. 임계치 MIN_SCORE=2 미만이면 Unknown.
- 스캔 대상: table 자신의 cell 텍스트 + owning paragraph 의 텍스트
  (heading-in-cell 또는 선행 heading 문단).
- `markdown_llm::emit_table` 이 `domain_hints` 시 TABLE 마커에 hint 추가.
  Unknown 은 attribute 자체를 빼서 noise 방지.
- CLI `--emit-domain-hints`. `--llm` 없이 쓰면 오류.
- TRL fixture 53 tables → 19 에 hint 붙음 (5 budget / 3 institution /
  4 performance / 7 personnel). 34 Unknown 으로 남음 (보수적·합리적).

**알려진 갭**:
- Schedule keyword 리스트가 TRL 에서 hit 안 남 (0개). 월별 일정이
  실제로는 gantt-style 숫자 셀로 구성되어 keyword 가 안 잡힘.
  단순 keyword 이상의 "숫자-heavy + 시간 패턴 컬럼" 감지가 필요.
- **Non-picture gso caption** (Phase 2b 갭).
- **Figure domain hint 미구현** — 현재는 TABLE 만. Figure 에도
  kind 가 있으면 유용 (예: architecture_diagram / product_shot /
  screenshot).

**다음 후보**:
- **Preview layer** (render crate) — IR → HTML with rowspan/colspan +
  폰트 fallback. 완전히 새 영역.
- **Non-picture gso caption** — line/rect/ole 캡션 살리기.
- **Fixture 확대** — TRL 외 heading-heavy / 수식-heavy / tracked-change.
- **Schedule heuristic** — keyword 밖의 구조적 감지 (다수의 월별
  컬럼 헤더 등).
- **Figure domain hint** — caption + context 기반.

**막힌 것**: 없음.

**작업 트리**: 깨끗. 테스트: 231/231 green.

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
