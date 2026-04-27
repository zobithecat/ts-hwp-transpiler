# 현재 위치

**지금** (2026-04-27): HWPX 측 round-trip이 단순 verbatim에서
**surgical header.xml rewriter** 단계로 진입. DocInfo IR mutation이
재읽기 결과에 살아남도록 보장. HTML preview는 *structural-only*로
스코프 확정 — 픽셀 fidelity는 `@rhwp/editor` iframe에 위임.

## 최근 ship (4월 마지막 주)

**HWPX writer DocInfo mutation flow** — `header_rewriter.rs`로
`Contents/header.xml`을 byte cursor 기반으로 surgical 편집:

- IR이 노출하는 attribute (`<hh:align horizontal>`, `<hh:charPr
  height/textColor/shadeColor/borderFillIDRef>`, `<hh:strikeout/underline
  shape+color>`, `<hh:font face>`, `<hc:winBrush faceColor>`) 만 IR값으로
  교체.
- 나머지 바이트(`<hh:styles>`, `<hh:numberings>`, `<hh:lineSpacing>`,
  `<hh:typeInfo>` Panose, `<hh:substFont>`, `<hh:bullets>`,
  `<hh:tabProperties>`, kerning 플래그 등)는 verbatim으로 통과 — 데이터
  손실 없음.
- 의도적 미지원: bold/italic 토글, 멀티스크립트 CharShape 배열, shape
  add/remove, gradation/image fill mutation.

**HTML preview 구조화 패스**:
- 위치 기반 stable IDs (`sec-{si}`, `par-s{si}-p{pi}`,
  `tbl-{path}`, `cell-{path}-r{r}c{c}`, `fig-{bin_id}`).
- `<section class="hwp-chapter hwp-lv-N">` 중첩 챕터 감싸기.
- ParaShape align → `text-align` CSS (justify/right/center/distribute).
- 헤딩 감지 폴백: 스타일 이름 매칭 외에 숫자 prefix (`1.`, `1.1.`,
  `(1)`, `①`, `가.` 등) 가드 포함.
- 그림 `aspect-ratio` 보존 + base64 data URI 인라인.

**WASM + 데모 마무리**:
- handle-based doc registry (62MB 파일 OK).
- rhwp-editor iframe lazy-load (탭 전환 후 첫 클릭 시).
- HTML/MD 미리보기 탭 + PDF/MD 다운로드 버튼.
- 레거시 이미지 포맷 (BMP/TIFF) JPEG 트랜스코딩.
- Hancom의 hex bin_id naming(`BIN{:04X}`) 호환.
- Equation script → LaTeX 변환기.

## 검증

- 408 tests green (codec + core + render + wasm).
- Round-trip 두 축 모두 통과:
  - 합성/실 fixture 바이트-동일 (HWP5 `unknown_streams` cache 경로).
  - HWPX 의미적 동일 + DocInfo mutation flow.
- 이번 세션에서 회귀 0건 (+12 신규 테스트).

## 알려진 갭

- **Phase 2 header rewriter**: 위 의도적 미지원 항목들. `crates/codec/
  src/hwpx/writer.rs` 모듈 docstring에 한계 목록 박힘.
- **MD → HWP/HWPX writer**: read 방향만 end-to-end. 새 문서를 IR에서
  쓸 때 일부 미타입 레코드는 hwplib 템플릿 번들에 의존
  (`blank_document`).
- **고충실도 렌더러**: 자체 구현 안 함 — rhwp iframe 위임 (2026-04-27
  결정).
- **HWP5 DEFLATE byte-equality**: flate2(miniz_oxide)는 hwplib(Java
  java.util.zip) 출력과 byte-equal 보장 안 함. structural equality로
  만족 — `project_deflate_byte_equal_ruled_out` 메모 참조.

## 다음 후보

1. **Phase 2 header rewriter 본격 구현** — bold/italic 토글 (`</hh:charPr>`
   직전 structural insert), 멀티스크립트 arrays overlay
   (`<hh:fontRef>` 등 7-script attr), paraPr/charPr add/remove, Fill IR
   타입화 + gradation/image overlay.
2. **MD → HWP 진짜 양방향 마무리** — 미타입 레코드 typed encoders
   (ID_MAPPINGS, NUMBERING, FACE_NAME emit, TRACK_CHANGE_*,
   LAYOUT_COMPATIBILITY) 대체로 hwplib 템플릿 의존 제거.
3. **TableBuilder 테스트 헬퍼** — `section_reencode.rs` cell-mutation
   포지티브 테스트 단순화.
4. **MD classifier 튜닝** — 1×1 wrapper 노이즈, value-heavy 분포.
5. **clippy 누적 경고 정리** + `emit_cell_line` struct 리팩토링
   (`#[allow(too_many_arguments)]` 제거).

## 막힌 것

없음. 작업 트리 깨끗, origin/master에 푸시됨 (HEAD `22f425c`).

## 빠른 컨텍스트

- 라운드트립이 1순위, 마크다운 quality는 2순위 (4월 22일 결정 유지).
- Fidelity render는 외부 위임 — rhwp iframe (4월 27일 결정).
- 스펙 사실은 `hwp5-spec-notes.md`, hwplib 포팅 맵은 `hwplib-mapping.md`.
- 저널 = "언제·왜 결정"; 이 문서 = "지금 무엇이 사실".
- HEAD 이동·테스트 변경·갭 변동 시 이 문서 업데이트 — 그게 존재 이유.
