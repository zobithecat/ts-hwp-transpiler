# 2026-06-30 — Hancom 렌더 충실도 디버그: itemCnt + lineseg + 셀너비, HWPX 소스 경로 확정

## 맥락

사용자가 실 문서(`06.23 연구개발계획서.대진대 작성.hwp`, 37쪽, 표·이미지 다수)로
`.hwp → md(--llm --split-assets) → .hwpx` 라운드트립을 한컴 오피스(mac)에서 검증.
초기 증상: **표 양식이 안 보이고 본문 문단이 세로로 겹쳐 까만 덩어리**. 한컴 뷰어가
유일한 계측기였고(헤드리스 렌더 불가), 사용자 스크린샷으로 단계별 추적.

결정적 전환점: 사용자가 원본 `.hwp`를 한컴에서 `.hwpx`로 "다른 이름으로 저장"해
**Hancom-authored 정답 HWPX**를 제공. 이걸 오라클로 구조 diff하면서 추측을 사실로 교체.

## 발견 / 결정

### 1. companion stem 페어링 footgun

`--split-assets`는 본문 `<stem>.md` + `<stem>.assets.md`(이미지 + **DOC_INFO 레이아웃
메타**)를 만든다. 사용자가 본문 md만 이름을 바꾸면 `md-to-hwpx`의 `<stem>.assets.md`
자동 페어링이 조용히 실패 → DOC_INFO 미적용 → section이 정의 없는 paraPr/charPr
id를 참조하는 dangling → 표/레이아웃 붕괴. (조용한 실패가 핵심 문제.)

### 2. `<hp:linesegarray>` 가 문단당 1개 seed만 → 문단 겹침

`section_writer`가 문단마다 `vertpos=0 vertsize=1000` lineseg 1개를 하드코딩(저널
2026-04-29 finding 6). 실제 줄 높이는 900/1100/1320 등 제각각이고 여러 줄로 wrap되는
문단도 많아, **캐시를 신뢰하는 한컴이 모든 줄·다음 문단을 같은 Y에 쌓음**. IR
`Paragraph.line_segments`(원본 PARA_LINE_SEG)는 있었으나 (a) writer가 무시, (b) MD가
운반 안 함 → 이중 손실. → `render_linesegarray`로 실제 segment를 모두 emit하고,
MD `lineseg=` 속성(`lineseg_codec`)으로 문단과 함께 운반(별도 id-키 불필요).

### 3. **(핵심) Hancom HWPX는 header refList 컨테이너에 정확한 `itemCnt`를 요구**

정답 HWPX diff 결과: `<hh:borderFills itemCnt="107">`, `<hh:charProperties
itemCnt="221">`, `<hh:paraProperties itemCnt="141">`, `<hh:styles itemCnt="40">`,
`<hh:fontfaces itemCnt="7">` — **모든 컨테이너에 itemCnt가 있다.** 우리 스켈레톤/
rewriter 출력은 **전부 누락**. itemCnt가 없거나 실제 자식 수와 다르면 **한컴이 그
컬렉션 전체를 거부하고 기본값으로 폴백** → 표 테두리·문자/문단 모양 전부 무시,
"완전 깨짐". borderFill 정의 자체는 멀쩡(SOLID 0.6mm)했는데도 컨테이너 헤더가
깨져 안 먹은 것. → `header_rewriter::inject_item_counts`가 최종 바이트에서 컨테이너별
실제 자식 수로 itemCnt를 **항상 재계산**(있으면 값 교체=동일 시 byte 보존, 없으면 삽입).
이 한 방으로 테두리·레이아웃이 살아남.

### 4. 셀 너비 미보존 → 라벨 열이 균등분배로 넓어져 배분정렬 글자가 퍼짐

MD가 셀 width를 안 실어 `cell_sizes::apply_defaults`가 페이지 폭을 열에 균등분배
(라벨 20% → 50%로 부풀음). CELL 레코드에 `width=`/`text_width=`를 운반해 실제 비율
복원. **height는 의도적으로 운반 안 함** — 한컴이 내용에 맞춰 행 높이를 자동 확장하는데
원본 laid-out 높이를 강제하면 표가 다음 페이지로 넘쳐 page-flow가 깨짐(실측 확인).

### 5. (결론) HWPX 소스 경로가 충실도 압도

`hwpx → md → hwpx`는 원본 header.xml(페이지 기하 left/right 5669·top/bottom 4251,
스타일, 테두리 정의)을 UNKNOWN_STREAM으로 **verbatim 보존** → 한컴 기본값을 역공학할
필요가 없다. 정답 HWPX 대비 **15개 stream 중 14개 byte-equal**(`section0.xml` 본문
포함; header.xml만 차이). 한컴 렌더가 원본과 사실상 동일. 사용자 확인 완료.

## 이유 / 트레이드오프

- `.hwp → hwpx` 직접 경로는 한컴이 안 들고 있는 정보(itemCnt 포맷, PAGE_DEF 여백,
  셀 기하)를 **전부 재생성**해야 한다 — 저널/CURRENT가 명시한 "가장 어려운 갭".
  itemCnt·lineseg·셀너비는 잡았으나 **페이지 여백(PAGE_DEF 미파싱)·page-flow**가 남아
  HWPX 소스만큼은 안 됨. 그래서 **사용자 권장 경로 = `.hwp`는 한컴에서 `.hwpx`로 저장
  후 `hwpx ↔ md`**로 확정.
- `inject_item_counts`는 "있으면 값 교체"라 unmutated 실 HWPX는 동일 값 → byte 보존
  → 기존 byte-equal 불변 테스트 유지(398 green).

## 결과 (이게 바뀌면 다시 볼 것)

- `header_rewriter::inject_item_counts` — 모든 header 생성/재작성 경로가 통과. 컨테이너
  추가/삭제 로직을 건드리면 itemCnt 재계산이 여전히 맞는지 확인.
- `lineseg_codec` — MD `lineseg=` 포맷(9필드 `:`구분, segment `|`구분). export/import 대칭.
- CELL 레코드의 `width=`/`text_width=` — `apply_defaults`는 width==0일 때만 채우므로
  운반값이 우선. height는 비운다(자동확장).
- throwaway였던 `hwp-to-hwpx-direct` bin은 제거. 직접 변환 path는 page-flow gap이
  남아 프로덕션 아님(저널 옵션 3 트랙으로 남김).

## 미해결 질문

- **HWP5 PAGE_DEF(tag 0x0049) 파싱** — 실제 페이지 여백/단 정보를 복원하면 `.hwp`
  직접 경로의 여백·page-flow를 맞출 수 있음. 현재 untyped passthrough.
- **표 page-flow** — 키 큰 표가 page 1에 못 들어가 다음 페이지로 밀리는 현상(DIRECT
  경로에서 관찰). secPr 페이지 기하가 정확해지면 자연 해소되는지 확인 필요.
- **multi-paragraph 셀의 2번째+ 문단** — 내용은 보존되나 셀 구조상 분리 처리 경로
  재확인 가치.
