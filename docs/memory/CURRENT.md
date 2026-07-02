# 현재 위치

**지금** (2026-07-02 심야): **표 앵커 paraShape 복원 + 텍스트 앵커 정렬 — 페이지 분할 해결.**
"기술개발 목표" 제목과 표가 두 페이지로 갈라진 원인: exporter 가 구조
문단(표/그림 앵커)에 PARAGRAPH 레코드를 안 찍어 **모든 표 wrapper 가
paraPr=0** 으로 임포트 — treatAsChar 표의 주변 간격은 앵커 paraShape 가
결정하므로 페이지 총높이가 변함. 수정: ① exporter 가 모든 문단에
PARAGRAPH 레코드 emit(앵커는 TEXT 없이 shape ids만), ② importer 가
TABLE[/FIGURE[ 직전의 PARAGRAPH 레코드를 wrapper 앵커 속성으로 소비,
③ verify-gate 정렬을 **텍스트 앵커 + 유계 resync 윈도우(5)** 로 교체
(위치 기반 걷기는 편집 1곳 이후 문서 전체의 pageBreak/빈 문단을 오배치),
④ 표 위치 이식에 frozen 앵커 shape 복사 추가(pageBreak 는 제외 — 깊은
중첩에서 1개 어긋날 수 있어 자기교정되는 정렬에 일임). 실측: pageBreak
8/8 원본 정위치, 표 앵커 paraShape 61/62, 대상 표 앵커 1/1. push 완료
(ad0914f). 잔여: 앵커 1개 paraPr 미세 불일치(중첩 구간 zip 슬립), blank
paraPr 일부 드리프트 — 시각 영향 미미.

(2026-07-02 밤): **md 왕복의 대형 데이터 손실 4종 수술 — 재조립 충실도 급상승.**
1페이지 완치 후 "다음 페이지들 약간 이상" 추적 → 원본 vs 재조립 요소 전수
비교로 발견: ① **멀티 문단 셀의 2번째+ TEXT 를 importer 가 조용히 폐기**
(서약서 조항·두 줄 제목 증발 — set_pending 이 pending 소진 후 drop),
② **셀 안 중첩 TABLE 이 바깥 표를 조기 종료**시키고 형제로 평탄화
(State::InTable 을 빌더 스택으로 재구성, 셀 내 FIGURE 도 동일 수정),
③ **빈 문단 551개 export 스킵** → 세로 리듬·페이지 흐름 붕괴 (이제
TEXT 없는 PARAGRAPH 레코드로 emit, flush 에서 실체화; 셀은 빈 body TEXT),
④ **pageBreak="1" 8개 미운반** (붙임/첨부 페이지 시작) →
`ParagraphHeader.page_break_before` 신설, HWPX↔IR↔md 왕복 (HWP5
divide_sort bit2 매핑). 추가: TEXT body 의 \n/\t 이스케이프 (lineBreak
꼬리 유실 방지). verify-gate 에 **align_paragraphs** — 동결 바이트와
2-포인터 정렬로 빈 문단 재삽입, pageBreak/styleIDRef 이식, lineBreak
분단 텍스트 복원, 표 셀 (row,col) 페어링 재귀 — 구버전 md 자동 업그레이드.
실측 회복: tc 1555/1555, tbl 62/62, p 1878→2331(원본 2429, 잔여는 AI 편집
드리프트), pageBreak 7/8. 잔여 갭(무해/저순위): 중간 colPr 8(전부
colCount=1 재선언 — 무해 확인), lineBreak 위치(소프트랩으로 대체),
fieldBegin/End·pageNum·caption 미운반, 문단 내 멀티 charshape run 평탄화.

(2026-07-02 저녁): **재조립 섹션의 페이지 지오메트리 verbatim 보존.**
linesegarray 생략 검증 중 사용자 발견: 원래 한 줄이던 문단이 재배치 후
줄바꿈 → 페이지 넘침. 문단 XML(paraPr/charPr)은 원본과 동일했고, 진범 후보는
**합성 secPr** — 여백 3000(원본 5669), 머리말/꼬리말 850(원본 2834),
gutterType LEFT_ONLY(원본 LEFT_RIGHT). 본문 폭이 다르면 재배치 시 모든 줄이
다시 감긴다. 수정: `Section.sec_pr_xml`(verbatim `<hp:secPr>` 조각) 신설 —
HWPX 파서가 바이트 스캔으로 캡처, writer 가 재조립 시 A4 합성 대신 splice,
LLM export 가 `SECTION[sec_pr=<base64>]` 로 운반(편집 모드), verify-gate 가
동결 바이트에서 회수(구버전 md 구제). 검증: 워크스페이스 그린, 실파일 재변환
출력의 margin/gutterType 원본 일치. 남은 확인: 한컴에서 ※ 문단 줄바꿈 재현
여부(docx/Word 비교는 폰트 치환 노이즈 있음 — hwpx 를 한컴에서 비교할 것).

(2026-07-02 오후): **재조립 렌더의 줄 겹침 해결 — linesegarray 생략 = 한컴 재배치.**
verify-gate 재조립 첫 실전 렌더에서 전 문단 줄 겹침 발생. 원인: HWPX 리더가
`<hp:linesegarray>` 를 IR 로 파싱하지 않아(동결 재생 경로라 그동안 불필요)
재조립 시 1,879개 문단 전부 vertpos=0 DEFAULT_LINESEG 폴백 → 한컴이 그대로
신뢰해 같은 위치에 적층. **실측 확정** (37쪽 실문서, 사용자 스크린샷):
linesegarray 를 **아예 생략하면 한컴이 줄 배치를 재계산**해 정상 렌더
(kordoc 의 "변경 섹션 라인 캐시 삭제" 트릭과 동일 원리). vertpos 는 리스트
(본문 흐름/셀) 내 **누적 좌표**라 편집 후엔 뒤따르는 모든 문단이 무효 —
부분 이식 불가, 섹션 단위 전체 무효화가 정답. 적용 3종: ① writer —
lineseg 없으면 `<hp:linesegarray>` 생략(DEFAULT_LINESEG 제거), ② verify-gate
— 편집 감지 시 섹션 전체 lineseg 재귀 wipe, ③ 편집 모드 export —
`lineseg=` attr 생략(어차피 무효+LLM 컨텍스트 절약). 아카이브(HWP5 소스)
export 는 lineseg carry 유지. 남은 트레이드오프: 재배치 후 페이지 흐름이
원본과 다를 수 있음(편집으로 내용이 늘면 당연) — 한컴에서 저장하면 정규화.

(2026-07-02 오전): **편집 안전장치 2종 — verify-gate + split 본문 단독 업로드 차단.**
실전 편집 테스트(260701 문서, AI가 시험평가 표 대량 수정)에서 표 서식 전멸
사고 발생. 원인은 코드가 아니라 워크플로 함정 2개: ① 웹앱에 split 본문
`.md` 를 **단독 업로드** → companion 의 DOC_INFO/header.xml 없이 스켈레톤
헤더 합성 → 모든 shape/borderFill 참조가 허공 → 서식 전멸(53KB hwpx).
② 페어로 올렸어도 아카이브 모드 export 의 `SECTION_BYTES` 가 편집을 조용히
무시했을 것. 수정: (a) **import verify-gate** (`import/markdown_llm.rs`) —
`SECTION_BYTES` 의 동결 XML 텍스트와 typed 본문 텍스트를 비교(공백/U+FFFC
무시), 다르면 `stream_bytes` drop → 편집이 항상 이김, 같으면 유지 →
byte-equal 보존. 이제 **아카이브 모드로 export 한 md 를 편집해도 재조립이
자동 발동**(--editable 몰라도 안전). HWP5 바이너리 캐시는 비교 불가라 제외.
(b) **웹앱 차단** (`ts/src/main.ts`) — llm 스탬프 있는데 assets 마커 없는
본문 단독 업로드를 명시적 에러로 거부. 검증: 18개 스위트 그린(byte-equal
픽스처 포함 = 게이트 오탐 없음), 사용자 실파일 CLI 재변환 6.1M 정상
(BinData 4, header 427KB verbatim, 편집 텍스트 반영, itemCnt 재계산).

(2026-07-01): **편집 워크플로 성립 — 두 모드 구분 확립.**
기본(아카이브) export 는 `SECTION_BYTES` 로 원본 섹션을 동결해 byte-equal
왕복을 만드는데, 이 때문에 **body md 편집이 무시됨**(writer 가 동결
바이트를 재생). 편집이 반영되려면 `--editable`(또는 `--edit-color`, 웹앱
토글)로 `SECTION_BYTES` 를 빼고 섹션을 문단 레코드에서 **재조립**해야 함
(→ 이 세션의 typed-emit 수정들: itemCnt·lineseg·cell-width·id=0 이 여기서
작동). 재조립 모드의 픽셀 충실도는 뷰어 검증 필요(freeze 모드의 byte-equal
렌더와 별개). 또 `--edit-color=#RRGGBB` 구현: doc_info 에 색 CharShape 추가
+ HWPX면 verbatim header.xml 에 charPr splice + 본문에 `edit-color` 마커
emit → 편집 에이전트가 `char_shape=` 를 그 id 로 바꿔 수정 문단을 색 표시.
`markdown_llm::inject_edit_color`. CLI end-to-end 검증(편집·색 반영 확인).

(2026-06-30): **HWPX 소스 라운드트립이 한컴 렌더 충실**. 실 문서
(`06.23 연구개발계획서`, 37쪽)에서 `hwpx → md → hwpx` 가 Hancom-authored
정답 HWPX 대비 **15개 stream 중 14개 byte-equal**(`section0.xml` 본문 포함),
한컴 오피스 렌더가 원본과 사실상 동일(사용자 확인). **권장 워크플로 확정**:
`.hwp` 는 한컴에서 `.hwpx` 로 저장 후 `hwpx ↔ md` 로 편집/복원.

이번 라운드 핵심 발견: **Hancom HWPX 는 header refList 컨테이너에 정확한
`itemCnt` 를 요구** — 없거나 실제 자식 수와 다르면 한컴이 그 컬렉션 전체를
거부하고 기본값 폴백(표 테두리·문자/문단 모양 전부 무시 → "완전 깨짐").
`header_rewriter::inject_item_counts` 가 최종 바이트에서 항상 재계산해 해결.
또 실제 lineseg(문단 겹침)·셀 width(열 비율)를 MD 가 운반하도록 확장. 상세는
저널 `2026-06-30-hwpx-itemcnt-and-roundtrip-fidelity.md`.

(이전 2026-04-29: MD 라운드트립이 HWPX 원본 기준 container byte-equal, `cmp`
exit 0. HWP5 → MD → HWPX cross-format 도 viewer 가 열고 그림 표시. 단 HWP5
source 의 표/body layout 은 doc_info 손실로 원본과 다름 — 저널
`2026-04-29-md-roundtrip-viewer-arc.md`.)

## 이번 라운드 ship (4월 29일)

전날 picture XML 8건 fix 직후, 사용자 viewer 검증으로 컨테이너·layout·
cross-format 결함 10개 발견. 단계별 ship:

- **UNKNOWN_STREAM 레코드** (`asset_footer.rs`, `markdown_llm.rs`):
  META-INF/, Preview/, settings.xml, version.xml 등 6개 stream을 footer
  에 base64 박아 라운드트립 보존.
- **header.xml reparse on import** (`markdown_llm.rs`): verbatim 으로
  복원된 header.xml 을 importer 가 다시 파싱해 doc_info 채움 →
  surgical rewriter 가 211/87/53 entry 모두 emit. 333KB → 64KB 손실
  사라짐.
- **bin_data picture-reference 순서 emit** (`writer.rs`): rhwp / mac
  HWP 2014 의 positional binding 에 맞춰 zip entry 순서 정렬.
- **manifest 깨끗하게 재작성** (`writer.rs`): 기존 dangling .jpg/.JPG
  `<opf:item>` 모두 strip 후 picture-reference 순서대로 깨끗한 항목만
  splice.
- **HWP5 cross-format guards** (`writer.rs`):
  - `looks_like_xml` sniff: `Section::stream_bytes` 가 binary OLE blob
    이면 typed emitter 로 fallback.
  - `is_hwpx_path` whitelist: HWP5 OLE 경로 (`/PrvImage`,
    `/Scripts/*`, `\x05HwpSummaryInformation` 등) 모두 drop.
  - skeleton 에 settings.xml + version.xml stub 추가.
- **`<hp:linesegarray>` per `<hp:p>`** (`section_writer.rs`): default
  Hancom 10pt-on-A4 lineseg 1개 emit. ~3987페이지 폭발 → 정상.
- **`binaryItemIDRef` lookup** (`section_writer.rs`): bin_id → manifest
  stem 매핑 thread. HWP5 source 의 `BIN0001` 형식도 정확히 reference.
- **`<hp:secPr>` 합성** (`section_writer.rs`): A4 portrait 페이지
  geometry 합성 paragraph prepend. viewer 가 페이지 dimension 을
  알 수 있게.
- **`<hp:tbl>` 4 layout 자식** (`section_writer.rs`): `<hp:sz>`/`<hp:pos>`
  /`<hp:outMargin>`/`<hp:inMargin>` 추가. cell 합으로 table extent
  계산.
- **`emit_new_para_pr` Hancom full child set** (`header_rewriter.rs`):
  `<hh:lineSpacing>` `<hh:breakSetting>` `<hh:margin>` `<hh:border>`
  `<hh:autoSpacing>` 모두 emit (이전엔 `<hh:align>` 한 줄).

## 검증

- 395 tests green (codec).
- HWPX 원본 라운드트립: container byte-equal. `[1.28...]` fixture 의
  13개 stream (mimetype, section0.xml, header.xml, content.hpf, BinData/*,
  META-INF/*, Preview/*, settings.xml, version.xml) 한 byte 도 다르지
  않음.
- HWP5 fixture (`260420-1...fin.hwp`, 5MB, 9 그림, 53 표): viewer 가
  열고 9개 그림 모두 표시. 페이지 수 정상화 (~3987 → 11). 단 표/body
  layout 은 원본과 다름 (아래 갭 참조).

## 알려진 갭

- **한글2018(구버전 미패치)에서 `="none"` 배경/음영을 검정으로 렌더** —
  2026-07-01 확인. HWPX 의 `faceColor="none"` / `shadeColor="none"`(배경
  없음)을 한글2018 편집화면이 검정으로 표시. **우리 파일 결함 아님**:
  문서 모델·인쇄·docx export·한컴2014 모두 정상이고, 우리 출력은 한컴
  원본과 15/15 byte-equal. 한컴 공식 **한글2018 패치 설치 시 정상**(알려진
  버전 호환 버그, 참고: shhh9461.tistory.com/198). 잠시 `none`→흰색/제거로
  우회하려 했으나 byte-equal 을 깨서 되돌림 — 정본은 무손실 유지, 미패치
  뷰어 대응은 필요 시 opt-in 으로만.
- **HWP5 → MD → HWPX 의 표/body layout** — 2026-06-30 대폭 개선. DOC_INFO
  레코드(doc_info JSON), 실제 lineseg, 셀 width, header itemCnt 까지 모두
  운반/재계산해 표 테두리·문단 겹침·열 비율 해결. **남은 갭**: HWP5 PAGE_DEF
  (tag 0x0049) 미파싱 → 페이지 여백이 하드코딩 기본값(3000/1417, 원본은
  5669/4251)이라 본문 폭/page-flow 가 원본과 다름. 키 큰 표가 다음 페이지로
  밀리는 현상도 여기서 옴. → HWP5 직접 경로의 픽셀 충실도는 아직 HWPX 소스
  경로에 못 미침. **권장은 HWPX 소스 경로**(원본 header verbatim 보존).
- **lossy 옵션 없음** — JPEG/WebP-lossy 로 더 줄일 수 있으나 round-trip
  안전성 깨짐. 의도적 미지원.
- **GFM split 모드 미구현** — GFM(human) 경로는 인라인 data URI 만 지원.
- **Phase 2 header rewriter** — bold/italic 토글, 멀티스크립트 CharShape
  배열, paraPr/charPr 추가/제거 됨. fontface add/remove 는 deferred.
- **HWP5 DEFLATE byte-equality**: flate2 vs Java Deflater. structural
  equality 만족.
- **고충실도 렌더러**: rhwp iframe 위임 (2026-04-27 결정).

## 다음 후보

1. **MD doc_info 인코딩** — `STYLES[id=N,name=...,paraPrIDRef=N,
   charPrIDRef=N]` / `PARA_SHAPE[id=N,...]` / `CHAR_SHAPE[id=N,height=...,
   bold=...]` 라인 레코드 추가. round-trip 시 IR `doc_info.styles`
   / `para_shapes` / `char_shapes` 채워서 layout 다양성 복원. (HWP5
   source 의 layout 깨짐 해결이 목표.)
2. **HWP5 → HWPX 직접 변환** — `Hwp5Reader → IrDocument → HwpxWriter`
   의 typed path 검증. MD 우회. 큰 스코프 변경.
3. **검증 자동화** — HWPX 라운드트립 byte-equality 를 fixture 기반
   regression test 로 강제.
4. **rhwp viewer 호환성 fixture 화** — `validation` 콘솔 출력의
   warning 종류별 fixture, 우리 emitter 가 어떤 attribute 를 빠뜨리는지
   추적.

## 막힌 것

없음. 작업 트리 깨끗, origin/master 푸시 완료.

## 빠른 컨텍스트

- 라운드트립이 1순위, 마크다운 quality 는 2순위 (4월 22일 결정 유지).
- Fidelity render 는 외부 위임 — rhwp iframe (4월 27일 결정).
- MD 에셋은 분리 default 권고 (4월 28일 결정).
- HWPX 라운드트립은 byte-equal — 4월 29일 마무리 단계 도달.
- HWP5 → MD → HWPX 의 layout 손실은 MD 포맷이 doc_info 를 throw away
  하기 때문 — 4월 29일 발견.
- 스펙 사실은 `hwp5-spec-notes.md`, hwplib 포팅 맵은 `hwplib-mapping.md`.
- 저널 = "언제·왜 결정"; 이 문서 = "지금 무엇이 사실".
- HEAD 이동·테스트 변경·갭 변동 시 이 문서 업데이트 — 그게 존재 이유.
