# 현재 위치

**지금**: Cell role classifier tuning 완료 — 연파란회색·연녹색 등
non-yellow pale accent 가 label 로 인식됨. TRL fixture label 수가
40 → 285 로 증가. L3 editable 규칙은 그대로 유지 (role=Label 이면
`false`).

**최근 ship**:
- `classify_roles` 확장: `is_label_tone` 헬퍼 추가, BgTone::Accent
  (Yellow/Green/Blue/Cyan/Magenta — Red 제외) 중 luminance ≥ 200 인
  것들을 label tone 으로 취급.
  - Yellow 는 luminance 무관 (기존 behavior).
  - Red 는 명시적 제외 (HWP 폼에서 경고/강조 색).
- Pale gray (`#E5E5E5`) first-col 셀이 label 패턴 문서 내에서 label
  로 승격.
- L3 (editable 추정): `role=value + 단일 paragraph + no controls +
  숫자 아님` → editable=true. 나머지 false/unknown.
- L2 + L3 + tuning 종합: TRL 1062 cells → 32 header / 285 label /
  745 value (415 editable + 330 non-editable value).

**알려진 갭**:
- **Non-picture gso 의 caption** (Phase 2b 갭).
- **`is_mostly_numeric` 임계치 90%** 는 경험값. 실제 사용 피드백
  으로 튜닝 가능.

**다음 후보**:
- **L4**: figure/caption 전역 domain hint (performance_metrics,
  budget 등). Heading keyword + 구조 패턴 기반 보수적 라벨.
- **Preview layer** (render crate) — IR → HTML with rowspan/colspan.
- **Non-picture gso caption** — line/rect/ole shape 의 caption 살리기.
- **Fixture 확대** — TRL 외 heading-heavy / 수식-heavy / tracked-
  change fixture.

**막힌 것**: 없음.

**작업 트리**: 깨끗. 테스트: 220/220 green.

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
