# 2026-04-22 — rowspan / colspan 방향 마커 확장 시도, 되돌림

**맥락**: 이전 저널 `2026-04-22-rounds-1-2.md` 가 "GFM은 rowspan 미지원,
row_span > 1이면 bullet 경로 강제"를 Open 질문으로 남김. TRL fixture의 §1
12×6 개요 표 전체가 bullet로 가버려 약 70줄의 `[r,c] span ...` 줄이 생겼고,
사용자가 편집 중 실제로 병합 구조에 헷갈렸던 경험을 공유.

**구현됨**: `try_build_md_grid` 에서 "row_span == 1 만" 조건을 제거하고
방향 마커로 확장 (커밋 `7c6f81a`, 이후 reset):

- `↑` — anchor 열, 그 아래 row
- `←` — anchor 행, 그 오른쪽 col
- `↖` — 사각형 병합 영역의 내부 corner

anchor 셀은 텍스트를 유지; continuation 셀은 마커를 채움. TRL fixture의
§1 12×6 개요 표가 bullet에서 단일 가독성 있는 grid로 변환되었으며 줄 수도
감소.

**되돌린 이유**: 결과가 시각적으로 좋았지만 프로젝트의 1순위 목표인
**라운드트립**을 깨는 것으로 확인됨:

- Continuation 셀에 `↑` 를 갖는 마크다운을 hwp로 역방향 변환 시 "이게
  병합 continuation인가 실제 `↑` 문자 내용인가"를 알 방법이 없음. 원본
  병합 재구성 불가.
- LLM 가독성 관점에서도 명시적인 `span 2×1` annotation이 더 기계 친화적.
  `↑` 마커는 인접성 추론이 필요 (즉 LLM이 방향을 infer해야 함).
- 사용자가 "병합 표가 어렵다"고 한 것은 *사람* 기준 불평이었지 LLM 기준이
  아니었음.

**결정**: `git reset --hard` 로 커밋 제거. 마크다운 소스는 좌표 annotation
(`[r,c] span N×M:`) 유지 — 무손실이며 LLM 친화적.

**결과**: 사람 친화적인 시각적 표 렌더링은 preview/renderer 레이어 책임.
렌더러는 HTML `<table>` 을 사용 가능 (`task.md` "No HTML tags" 규칙은
마크다운 export에만 적용, 미리보기 렌더링에는 적용 안 됨). 이 방식으로 세
목적이 동시에 충족됨:

- 마크다운 소스 — 무손실, LLM 친화, 좌표 명시
- 미리보기 렌더 — 사람 친화, HTML table 재조립 가능
- 라운드트립 — 손상 없음 (span 정보가 md에서 유지되므로)

**미해결**: preview/renderer crate 미구현 — land할 때 `span N×M`
annotation을 HTML table로 재조립해야. 그게 지금 Open으로 남은 시각적
렌더링의 목적지.
