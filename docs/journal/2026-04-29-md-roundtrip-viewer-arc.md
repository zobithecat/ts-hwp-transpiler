# 2026-04-29 — MD 라운드트립 viewer 호환성 디버그 (HWPX & HWP5 source)

## 맥락

전날(`2026-04-28-image-round-trip-debug.md`) 8건의 picture-emit 결함을
잡고 push한 직후, 사용자가 (1) HWPX 원본 → MD → HWPX 라운드트립과
(2) **HWP5(.hwp) 원본 → MD → HWPX 변환** 두 경로를 viewer에서 검증하며
연속 보고. 이전 라운드는 picture XML 자체의 결함이었지만 이번 라운드는
**컨테이너·layout·cross-format** 결함이 줄줄이 드러남. 그 디버그 사이클의
영구 기록.

테스트 fixture:
- `[1.28.수+석간]+장애인+정보접근권+...무인정보단말기.hwpx`
  (HWPX 원본, 그림 2장, 116KB)
- `260420-1. 연구개발계획서(서식)[TRL점프업 1단계]_대진대_수정_1430_fin.hwp`
  (HWP5 원본, 그림 9장, 표 53개, 5MB)

## 발견 1 — UNKNOWN_STREAM record 미존재 → MD 라운드트립이 6개 stream 통째 손실

`<!-- hwp-transpiler: assets -->` footer는 ASSET / SECTION_BYTES만 emit.
HWPX 컨테이너의 META-INF/container.rdf, META-INF/manifest.xml, Preview/*,
settings.xml, version.xml은 importer가 복원할 길이 없어 round-trip 시 통째
사라짐. Hancom viewer는 이들 부재 시 picture/line-layout 모두 거부.

**fix**: footer에 `UNKNOWN_STREAM[name=...,len=N]` + `DATA: data:application/
octet-stream;base64,...` 레코드 추가. importer가 base64 디코드 후
`unknown_streams.insert(name, bytes)`. 13개 stream 모두 byte-equal 보존.

## 발견 2 — header.xml 333KB → 64KB 손실

UNKNOWN_STREAM이 header.xml byte를 보존해도 importer의 `doc.doc_info`는
synthesised heading slot 7개만 갖고 있음. surgical header rewriter가
`id ≥ IR.len(7)`인 charPr/paraPr/style을 모두 truncate → header가
64KB로 쪼그라듦.

**fix**: importer의 main loop 종료 후 `unknown_streams["Contents/header.xml"]`
를 `parse_header_xml`로 다시 파싱해 `doc.doc_info`에 211개 charShape /
87개 paraShape / 53개 style을 모두 채움. rewriter가 IR ↔ verbatim 동일
view로 동작 → 333,660 byte 그대로 emit.

## 발견 3 — picture/zip-entry/manifest 순서 mismatch

원본 zip이 `BinData/image2.png` 먼저, `image1.png` 다음으로 저장되어
있고 manifest도 dangling `image1.jpg`/`image2.JPG` 항목까지 4개를
중복 emit. picture XML은 `binaryItemIDRef="image1"` 첫 번째로 참조.

세 layer가 어긋나 viewer가 binding을 viewer-specific하게 넘김:
- 한컴: manifest id-lookup → 정상
- rhwp / mac HWP 2014: positional binding → image1 ↔ image2 swap

**fix 1** (writer.rs): `bin_data_in_picture_order`로 picture-reference
순서대로 zip emit (orphan 끝에).

**fix 2** (writer.rs): manifest 1차 패스에서 모든 BinData `<opf:item>`
strip, 2차 패스에서 picture-reference 순서대로 깨끗한 항목만 splice.
dangling .jpg/.JPG 참조 영구 제거.

검증: HWPX 원본 → MD → HWPX 라운드트립이 zip 컨테이너 byte-equal까지
도달.

## 발견 4 — HWP5 reader가 OLE stream을 unknown_streams 로 surface

`.hwp` 입력은 OLE compound 파일. reader가 `\x05HwpSummaryInformation`,
`/PrvImage`, `/Scripts/DefaultJScript`, `/DocOptions/_LinkDoc` 등을
`unknown_streams`에 그대로 적재. HWPX writer는 그걸 `BinData/`/section/
mimetype/ Contents/header.xml만 빼고 verbatim passthrough → OLE 경로가
HWPX 패키지에 박혀 viewer가 거부.

**fix**: `is_hwpx_path` whitelist (mimetype, settings.xml, version.xml,
Contents/, BinData/, META-INF/, Preview/, Scripts/, Charts/) 으로 필터.
HWP5 OLE 경로 모두 drop.

## 발견 5 — HWP5 binary section blob을 Contents/section0.xml로 dump

HWP5 reader는 `BodyText/Section{N}` binary OLE blob을 `Section::stream_bytes`
로 캐시. HWPX writer는 verbatim cache 패스를 무조건 신뢰해 그 blob을
`Contents/section0.xml`에 통째 박음. 첫 non-tag 바이트에서 XML 파서가
EOF 에러 (`tag not closed: > not found`).

**fix**: `looks_like_xml` sniff (whitespace + UTF-8 BOM 스킵 후 `<` 인지).
binary blob이면 typed XML emitter 경로로 fallback.

## 발견 6 — 모든 paragraph가 lineseg=0 → 1 페이지 = 1 줄로 폭발

typed `section_writer`가 `<hp:linesegarray>` 자체를 emit 안 함. viewer가
`line_height=0`으로 처리 → 5쪽 문서가 ~3987 페이지로 reflow 시도.

**fix**: 각 `<hp:p>` (top-level + cell-nested) 끝에 default lineseg
1개 emit. Hancom 10pt-on-A4 기본값 (vertsize/textheight=1000, baseline=850,
spacing=600, horzsize=42520, flags=393216). rhwp `auto-fix`/한컴 textRun
reflow가 실측 geometry로 다시 계산.

## 발견 7 — `binaryItemIDRef="image{N}"` 하드코드

`emit_picture`가 source format 무시하고 항상 `image{N}` reference. HWP5
source의 BinData는 `BIN0001.png` 형식 → reference / manifest id /
filename 셋이 mismatch → viewer가 picture를 못 찾음.

**fix**: `write_section_xml` 진입 시 `bin_data` 에서 `bin_id → manifest
stem` lookup table 빌드, `emit_paragraph → emit_run_with_range →
emit_picture / emit_table → emit_cell` 체인 통해 thread. `emit_picture`
는 lookup 으로 실제 stem 사용 (HWPX `image1`, HWP5 `BIN0001`).

## 발견 8 — `<hp:secPr>` 미존재 → viewer가 페이지 dimension 못 정함

typed emitter가 page geometry를 전혀 안 박음. 한컴 HWP 2014 / mac viewer
는 secPr 없으면 거부, rhwp는 0-margin으로 fallback.

**fix**: section 시작에 합성 secPr paragraph 1개 prepend. A4 portrait
(59528×84188 HWPU), 25mm sides + 15mm top/bottom 마진, footnote/endnote/
pageBorderFill 기본 stub.

## 발견 9 — `<hp:tbl>` 4 layout 자식 누락

Hancom-authored `<hp:tbl>`는 opening tag 직후 `<hp:sz>` (bounding box) /
`<hp:pos>` (anchor) / `<hp:outMargin>` / `<hp:inMargin>` 4개 자식을
반드시 emit. 우리는 그냥 `<hp:tr>` 로 직진 → viewer가 표 dimension 모름.

**fix**: 셀 width 합 (row 0 기준), 셀 height 합 (행별 max) 으로
table extent 계산. `<hp:sz>`/`<hp:pos>`/outMargin(283/283/283/283)/
inMargin(141/141/141/141) emit. 표 레벨 `borderFillIDRef`도 하드코드
"0" 에서 `t.border_fill_id`로 교체.

## 발견 10 — `emit_new_para_pr` 가 `<hh:align>` 한 줄만 emit

`header_rewriter::emit_new_para_pr` (IR-side ParaShape 으로부터 새 paraPr
생성) 가 alignment만 박음. lineSpacing / breakSetting / margin / border /
autoSpacing 모두 누락 → viewer가 paragraph layout을 못 잡고 그 안의
표/그림도 같이 깨짐.

**fix**: Hancom-typical full child set (`tabPrIDRef` / `condense` /
`fontLineHeight` / `snapToGrid` / `suppressLineNumbers` / `checked` 속성 +
`heading` / `breakSetting` / `margin` / `lineSpacing 160% PERCENT` /
`border` / `autoSpacing` 자식). 기본값은 Hancom 의 plain body text 규격.

## 결과

코드 path 별로 단계적 ship. 이 라운드 완료 시점:
- **HWPX 원본 라운드트립**: container byte-equal (`cmp` exit 0).
  `[1.28...]` fixture 는 13개 stream 한 byte도 다르지 않음.
- **HWP5 → MD → HWPX 변환**: viewer 가 *열고* 9개 그림 모두 표시
  (이전엔 안 열리거나 그림 missing). 페이지 수 정상화 (~3987 → 11).
- **테스트**: 395 passed (18 suites).

## 미해결 — HWP5 → MD → HWPX 의 표/layout 결함

증상 (사용자 보고, 데모 viewer):
- "그림은 나오고 표는 깨짐" — 표 dimension 은 맞지만 cell 안 텍스트
  배치, 행 높이 제어가 깨짐.
- "레이아웃도 어긋남" — 페이지 11쪽으로 정상화됐지만 본문 흐름이
  원본과 다름.

근본 원인 (검증된 가설):
- **MD 포맷이 doc_info detail을 거의 다 버림**. HWPX 원본 라운드트립은
  UNKNOWN_STREAM 으로 header.xml verbatim 보존 → reparse 로 211 char_shape /
  87 para_shape / 53 style 복원. 하지만 HWP5 source는 header.xml 자체가
  존재하지 않음 (HWP5는 binary record stream).
- HWP5 reader가 doc_info 를 채워도 `to_llm_markdown` 이 그걸 ASCII 노테이션으로
  re-emit 안 함. importer의 `style_synth` 가 heading 시 7개만 합성.
- 결과 모든 paragraph 가 `paraPrIDRef="0"` / `charPrIDRef="0"` / `styleIDRef="0"`
  참조. 한 ID 가 모든 paragraph 의 layout 을 결정 → 표·body·heading 이
  같은 metric 으로 렌더.

가능한 다음 단계 (선택지):
1. **MD 포맷 확장** — `STYLES` / `PARA_SHAPES` / `CHAR_SHAPES` 레코드 추가해서
   HWP5 → MD 시 doc_info 를 base64-frozen blob 또는 typed line records 로 emit.
   importer가 round-trip 복원. 큰 스코프 변경.
2. **Skeleton 의 default id=0 enrich** — 부분적으로 해서 `emit_new_para_pr`
   는 이미 풀 child set 출력. 나머지 (charPr, style) 도 동일하게 enrich.
   하지만 *모든* paragraph가 동일 id=0 referencing 인 한 진짜 다양한 layout
   복원은 불가.
3. **HWP5 → HWPX 직접 변환** — MD 우회. HWP5 binary records 를 typed
   emitter 가 HWPX XML 로 바로 변환. 큰 스코프 변경, 별도 트랙 가치 있음.

## 미해결 질문

- 사용자 시나리오에서 "MD 우회 path 가 우선 필요하냐" — `.hwp 업로드 →
  HTML 미리보기 / .md 추출` 만 쓰는 거라면 MD 라운드트립 자체가 필요
  없고, .hwp → .hwpx 직접 변환만 이슈. .md 를 LLM 으로 편집 후 다시
  .hwpx 가 워크플로면 MD 포맷 doc_info 확장이 필수.
- rhwp viewer 의 strict validation 출력 — 내부 모델은 black box 라
  어떤 attribute / 자식이 부족해 layout 깨지는지 추적 어려움. 데모의
  `[validation]` 콘솔 로그가 유일 단서.

## 코드 변경 위치

- `crates/codec/src/export/asset_footer.rs` — UNKNOWN_STREAM emit
- `crates/codec/src/import/markdown_llm.rs` — UNKNOWN_STREAM/SECTION_BYTES
  decode + header.xml reparse
- `crates/codec/src/hwpx/writer.rs` — looks_like_xml sniff,
  is_hwpx_path filter, bin_data_in_picture_order, manifest pass-1/pass-2
- `crates/codec/src/hwpx/section_writer.rs` — secPr 합성, lineseg
  default, 표 4 layout child, BinLookup thread
- `crates/codec/src/hwpx/skeleton.rs` — settings.xml + version.xml stub
- `crates/codec/src/hwpx/header_rewriter.rs` — `emit_new_para_pr`
  Hancom-typical full child set
