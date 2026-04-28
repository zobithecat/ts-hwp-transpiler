# 2026-04-28 — MD 에셋 파이프라인 (5단계)

## 맥락

`.hwpx → MD → .hwpx` 라운드트립에서 **그림이 사라지는 문제**가 사용자 보고로
드러남. 자아비판 결과 두 갈래 손실:

- 데모의 `.hwpx 다운로드` 버튼이 항상 MD를 거쳐 가서, HWP/HWPX 업로드의
  `bin_data`가 round-trip 도중 평탄화됨 → 직전 커밋(75f33b9)에서
  ourIr를 직접 saveHwpx에 넘기는 단순 passthrough로 수정.
- 진짜 round-trip 워크플로(LLM이 본문 수정 → MD를 다시 .hwpx로)에서는
  이미지가 MD에 실리지 않으면 본질적으로 살릴 길이 없음. → 본 저널의
  주제: MD에 이미지를 어떻게 실을 것인가.

## 결정 — 분리 .md 페어가 default, 인라인 base64는 옵션

설계 토론(이 저널에 응축):

| | 인라인 footer | 분리 .md 페어 |
|---|---|---|
| 본문 깨끗 | △ | ✓✓ |
| **LLM 컨텍스트 절약** | ✗ | **✓** |
| 단일 아티팩트 | ✓ | ✗ |
| 라운드트립 | ✓ | ✓ (페어링 필요) |

기각된 대안:
- **Blob URL 매칭** — 세션 메모리 한정. 다른 세션/도구로 MD를 옮기면 깨짐.
  진짜 round-trip이 아닌 세션 캐시일 뿐.
- **사이드카 zip** — 압축/해제 코드 추가, UX 복잡. 두 .md 파일이 더 단순.

채택 — **두 모드 모두 지원, 분리를 default 권고**:
- LLM 워크플로에 압도적으로 유리("doc.md만 LLM에 보내고, .assets.md는 보존")
- 단일 자족 파일이 필요하면 `--inline-assets`로 인라인 base64 footer 가능

## 결정 — 리사이즈 + 재인코딩 정책

원본 바이트 그대로 base64 인코딩하면 MB 폭증. 그래서:

- **72 DPI 기본 리샘플** (1 inch = 72 px = 7200 HWPUNIT). HWP가 advertise하는
  width_mm/height_mm을 화면-스케일 픽셀 dim에 일치시킴.
- **36 DPI 옵션** (LLM 컨텍스트 더 줄이고 싶을 때, 절반 픽셀)
- **lossless PNG 재인코딩** — `image` crate 내장. 알파 채널 보존, 디코드된
  바이트가 round-trip 시 drift 안 함.

JPEG/lossy WebP 기각: round-trip의 의미는 "원본 ≈ 재import 결과"인데, lossy는
이걸 깨뜨림. 파일 크기는 PNG로 충분히 절약됨(72→36 DPI 절반).

## 채택된 포맷

본문 측 LLM 모드:
```
FIGURE[id=fig-1,bin_id=1,width_mm=120,height_mm=80,asset_ref=asset-1]
```
(asset_ref는 향후 cross-reference용; 현재는 bin_id가 1차 키)

본문 측 GFM 모드:
```
![](data:image/png;base64,iVBORw0K...){width=120mm; height=80mm}
```
(인라인 인코딩만; CommonMark 표준)

에셋 footer (LLM/GFM 공통):
```
<!-- hwp-transpiler: assets -->

ASSET[id=asset-1,bin_id=1,mime=image/png,width=W,height=H,dpi=72,source_id=image1.png]
DATA: data:image/png;base64,iVBORw0K...
```

분리 모드는 `<stem>.assets.md` 컴패니언 파일로 같은 footer 내용 분리.
첫 줄에 `<!-- hwp-transpiler: format=assets -->` 마커.

## 5단계 구현 결과

1. `crates/codec/src/asset_pipeline.rs` — encode/decode + resize + base64
   공통 헬퍼. 11 unit test.
2. LLM exporter/importer split + inline. `<!-- hwp-transpiler: assets -->`
   footer + `ASSET[...]` / `DATA:` 레코드. FIGURE → PictureControl.
   `tests/asset_round_trip.rs` 4 통합.
3. GFM 측 inline `![](data:image/png;base64,...)` exporter +
   pulldown-cmark `Tag::Image` event → PictureControl 임포터. 1 통합.
4. CLI 플래그: `--inline-assets` / `--split-assets` / `--asset-dpi=N`.
   `md-to-hwpx`가 `<stem>.assets.md` companion 자동 검출 → 본문에 concat.
5. 데모: `<input type="file" multiple>` + asset-mode/DPI select +
   페어 `.md` + `.assets.md` 자동 매칭 + split 모드 시 두 파일 다운로드.

검증: `sample.hwpx` (3.1MB, 그림 3개) → `--llm --split-assets` →
`sample.md` (99KB) + `sample.assets.md` (3.9MB) → `md-to-hwpx sample.md`
(companion 자동 검출) → 출력 .hwpx에 BinData/image{1,2,3}.png 모두 복원.

## 미해결 질문

- **반응형 height 계산** — 현재 `width_mm`/`height_mm`만 mm→HWPUNIT로 변환.
  실제 viewer에서 보이는 비율이 살아남는지는 별도 fixture로 확인 필요.
- **Lossy 옵션** — JPEG/WebP-lossy를 toggle로 추가하면 파일 크기 더 줄지만,
  round-trip 안전성 보장 안됨. 현재는 의도적으로 미지원.
- **여러 섹션의 그림** — 현재 `bin_data`는 doc-global. 한 .hwpx의 모든
  섹션 그림이 한 파일의 footer에 모음. 섹션별 분리는 미고려.
