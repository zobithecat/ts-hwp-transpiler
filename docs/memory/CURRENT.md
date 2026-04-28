# 현재 위치

**지금** (2026-04-28): MD ↔ HWPX 라운드트립이 **이미지까지 살림**. 5단계
asset 파이프라인 ship — encode/decode 공통 헬퍼 + LLM/GFM exporter/importer
양쪽 + CLI `--inline-assets` / `--split-assets` 플래그 + 데모 페어
`.md` / `.assets.md` 업로드/다운로드 토글.

## 이번 라운드 ship (4월 28일)

**MD 에셋 파이프라인 (5 phase, `d66383e`~`ebb7be9`)**:

- `asset_pipeline` 모듈 — 원본 그림 바이트를 72/36 DPI로 리샘플 + lossless
  PNG 재인코딩 + base64로 wrap. inverse decode도 같은 모듈.
- 두 emit 모드:
  - **Inline**: 본문 끝에 `<!-- hwp-transpiler: assets -->` footer +
    `ASSET[...]` / `DATA: data:image/png;base64,...` 레코드. 단일 자족 .md.
  - **Split** (default 권고): `<stem>.md` (본문, 깨끗) + `<stem>.assets.md`
    (에셋만). LLM 컨텍스트 절약 워크플로 친화.
- LLM 노테이션 `FIGURE[id=fig-N,bin_id=N,width_mm=W,height_mm=H]` 본문
  레코드. importer가 PictureControl로 복원.
- GFM(human) 모드는 인라인 data URI만 지원 (CommonMark `![](data:...)`).
- CLI: `hwp-to-md --split-assets` → companion 자동 emit. `md-to-hwpx`가
  `<stem>.assets.md` 자동 검출.
- 데모 UI: 에셋 모드 select(텍스트만/인라인/분리) + DPI select(72/36) +
  multi-file picker로 페어 업로드 지원 + 분리 모드 시 두 파일 자동
  다운로드.

**직전 fix들 (4월 27일)**:

- 데모 `.hwpx 다운로드` 가 ourIr 직접 saveHwpx — MD round-trip 강제 X
  (`75f33b9`). HWP/HWPX 업로드의 그림이 single click에선 살아남음.
- skeleton borderFill에 `<hc:fillBrush>` 추가, id=0/1 둘 다 정상 자식
  구조 갖춤. 셀 테두리 viewer에서 보임.
- HWPX `<hh:styles>` 컨테이너 reader/writer 와이어. styleIDRef →
  `Paragraph::header.style_id` 와이어. heading 라운드트립 (`# 제목`).

## 검증

- 490 tests green (codec + core + render + wasm).
- `sample.hwpx` (3.1MB, 그림 3개) round-trip:
  `hwp-to-md --llm --split-assets` → `sample.md` (99KB) +
  `sample.assets.md` (3.9MB) → `md-to-hwpx sample.md`(companion 자동 페어)
  → BinData/image{1,2,3}.png **세 그림 전부 복원**.
- 데모(npm run dev)에서 .hwpx 업로드 → "분리" 모드 토글 → .md 다운로드 →
  자동으로 .md + .assets.md 두 파일 받음 → 두 파일 같이 업로드 → 페어 인식
  → .hwpx 다운로드로 그림 살아남는지 확인 가능.

## 알려진 갭

- **lossy 옵션 없음** — JPEG/WebP-lossy로 더 줄일 수 있으나 round-trip 안전성
  깨짐. 의도적 미지원.
- **GFM split 모드 미구현** — GFM(human) 경로는 인라인 data URI만 지원.
  Split 모드는 LLM 노테이션에서만 의미 있음 (CommonMark는 reference-style
  외에 footer 표준 없음).
- **그림 비율 viewer 검증 미완** — width_mm/height_mm는 mm→HWPUNIT 변환
  살아남지만, 실제 HWP/rhwp 뷰어에서 보이는 비율이 정확한지 별도
  fixture로 검증 필요.
- **Phase 2 header rewriter** — bold/italic 토글, 멀티스크립트 CharShape
  배열, paraPr/charPr 추가/제거는 이미 됨(이전 세션). gradation/image fill
  mutation, fontface add/remove는 여전히 deferred.
- **HWP5 DEFLATE byte-equality**: flate2 vs Java Deflater. structural
  equality 만족.
- **고충실도 렌더러**: rhwp iframe 위임 (2026-04-27 결정).

## 다음 후보

1. **검증 라운드** — 분리/인라인 모드의 실제 HWP 뷰어 호환성 fixture로
   체크. width_mm/height_mm 비율이 깨지는지 시각 확인.
2. **non-heading 구조 스타일 round-trip** — 조/항/호 같은 legal-doc 스타일.
   PARAGRAPH 레코드에 `style_name=...` 확장 추가.
3. **clippy + dead-code 정리** — 누적된 warning 정리.
4. **`emit_cell_line` struct 리팩토링** — `#[allow(too_many_arguments)]`
   제거.

## 막힌 것

없음. 작업 트리 깨끗, origin/master HEAD `ebb7be9`까지 푸시.

## 빠른 컨텍스트

- 라운드트립이 1순위, 마크다운 quality는 2순위 (4월 22일 결정 유지).
- Fidelity render는 외부 위임 — rhwp iframe (4월 27일 결정).
- MD 에셋은 분리 default 권고 (4월 28일 결정 — 이 저널 참조:
  `2026-04-28-md-asset-pipeline.md`).
- 스펙 사실은 `hwp5-spec-notes.md`, hwplib 포팅 맵은 `hwplib-mapping.md`.
- 저널 = "언제·왜 결정"; 이 문서 = "지금 무엇이 사실".
- HEAD 이동·테스트 변경·갭 변동 시 이 문서 업데이트 — 그게 존재 이유.
