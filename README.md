# ts-hwp-transpiler

브라우저 네이티브 미리보기를 갖춘 **HWP/HWPX ↔ 마크다운** 양방향
트랜스파일러. 목표는 한국어 오피스 문서를 마크다운으로 라운드트립
시키되 구조 손실 없이 처리하고, 결과물을 일반 MD (LLM·에디터용) 또는
고충실도 미리보기 (사람용) 두 형태로 모두 렌더링하는 것.

> 상태: 초기 — reader는 읽기 경로를 잘 다루고 writer는 verbatim 캐시로
> 라운드트립하며, 마크다운 export는 테스트 코퍼스에서 충분히 사용 가능.
> WASM과 미리보기 레이어는 스텁 상태. 현재 진행 상황은
> `docs/memory/CURRENT.md` 참고.

## 구성

```
crates/
  core/          중립 IrDocument + Reader/Writer trait
  codec/         HWP5 reader/writer + 마크다운 exporter (코드 대부분 여기)
  render/        백엔드 무관 RenderCommand 골격 (canvas/SVG 준비됨)
  wasm/          브라우저 미리보기를 위한 wasm-bindgen surface
docs/
  memory/        라이브 레퍼런스 문서 (현재 상태, 스펙 노트, hwplib 포팅 맵)
  journal/       append-only 설계 로그 (결정·이유·되돌림 기록)
test/            개인 fixture (.gitkeep 외 gitignored)
```

## 빠른 시작

```sh
# 워크스페이스 전체 빌드 + 테스트
cargo test --workspace

# HWP → 마크다운
cargo run -p hwp-transpiler-codec --bin hwp-to-md -- path/to/input.hwp
# 기본은 ./path/to/input.md 로 출력; `-` 인자는 stdout
```

`crates/codec/tests/fixtures/` 에 neolord0/hwplib 의 작은 샘플
fixture가 vendor 되어있음 (Apache 2.0). 라운드트립 스위트는 이 vendor된
파일과 `/test/` 에 직접 둔 개인 HWP를 모두 사용.

## 현재 동작하는 것

- **/FileHeader, /DocInfo, /BodyText/Section{N}** — 타입드 read +
  verbatim 스트림 캐시를 통한 바이트 동일 라운드트립 (mutate된 record는
  re-encode 트리거).
- **DocInfo 레코드** 타입화: `DocumentProperties`, `IdMappings`,
  `FaceName ×7 슬롯`, `BorderFill`, `CharShape`, `ParaShape`,
  `Style`, `BinData`. 미타입 태그는 `raw_records`로 무손실 통과.
- **BodyText 레코드** 타입화: paragraph header / text / char-shape
  runs / line segments, `LIST_HEADER` (cell), `TABLE` + cells,
  그리고 `gso → SHAPE_COMPONENT → SHAPE_COMPONENT_PICTURE` 체인을
  `ControlKind::Picture` 로.
- **임베디드 바이너리** — `/BinData/<id>.<ext>` 스트림이
  `IrDocument.bin_data`로 들어가며 MIME 자동 결정.
- **마크다운 export** — 여러 단계 품질 개선: 헤딩 감지,
  colspan 그리드 확장, 단일행 박스 표 → 본문 변환, wrapper 표 unwrap,
  긴 셀 nested sub-bullet, 빈 셀 range 압축, 한컴 PUA 글머리표
  (`󰊱` 등) → 표준 `①..⑳` 정규화. 병합 셀 정보는
  `[r,c] span N×M:` annotation으로 무손실 보존 (시각적 렌더링은
  미리보기 레이어 책임).

TRL R&D 계획서 fixture (5 MB, 53 표, 9 임베디드 이미지) 기준으로
바이트 동일 라운드트립 + 가독성 있는 마크다운 export 모두 통과.

## 아직 없는 것

- **마크다운 writer** (md → hwp) 가 다음 큰 단계. 현재는 read 방향만
  end-to-end; writer surface는 verbatim 캐시 통과.
- **WASM 브라우저 미리보기** — `crates/wasm`, `crates/render` 가
  스캐폴드는 있지만 실제 렌더러 미구현.
- **이미지 마크다운 emit** — 그림은 IR로 파싱되지만 CLI가 아직
  사이드카 파일 (`<doc>.assets/`) 을 dump 안 함; `MdOptions
  .assets_path` 는 있고 wiring 진행 중.
- **캡션, 수식, 각주, 변경 이력 등** — verbatim 보존되지만 표면화 안 됨.

## 문서

- **`docs/memory/CURRENT.md`** — *지금* 무엇이 사실인가. 새 세션
  시작할 때 가장 먼저 읽을 것.
- **`docs/memory/hwp5-spec-notes.md`** — 통합 binary-format 사실
  모음 (HWP5 공식 스펙은 여러 영역이 불완전 — 우리가 실제로 검증한 것).
- **`docs/memory/hwplib-mapping.md`** — Java
  [hwplib](https://github.com/neolord0/hwplib) → Rust 파일 매핑.
- **`docs/journal/`** — 설계 결정·기각된 대안·되돌림 기록.
  해당 디렉토리의 README가 entry 템플릿 설명.
- **`task.md`** — 원래 미션 스펙.

## 라이선스

(미정 — TBD)
