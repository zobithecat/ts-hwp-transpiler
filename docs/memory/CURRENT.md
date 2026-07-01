# 현재 위치

**지금** (2026-06-30): **HWPX 소스 라운드트립이 한컴 렌더 충실**. 실 문서
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
