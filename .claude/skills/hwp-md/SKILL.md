---
name: hwp-md
description: 트랜스파일된 구조화 Markdown(.md) 읽기·편집·복원 워크플로. HWP/HWPX 문서를 md로 변환해 읽거나, LLM 규칙에 따라 본문을 편집한 뒤 다시 .hwpx로 복원할 때 사용. Trigger — "hwp를 md로", "변환된 md 읽어/고쳐", "md를 다시 한글로", roundtrip 요청.
---

# hwp-md — 트랜스파일된 Markdown 읽기 · 편집 · 복원

이 저장소의 트랜스파일러가 만든 **구조화 Markdown(format=llm)** 은
`.hwpx` 무손실 왕복을 위한 중간 표현이다. 레코드 줄을 건드리면 원본
양식(표·서식·레이아웃)이 깨지므로 반드시 이 워크플로를 따른다.

## 1. 변환 (hwp/hwpx → md)

```sh
# 읽기 전용 (아카이브 모드, byte-equal 왕복 — 편집 반영 안 됨)
cargo run -p hwp-transpiler-codec --bin hwp-to-md -- --llm doc.hwpx doc.md

# 편집 목적 (편집이 실제로 .hwpx에 반영되는 모드)
cargo run -p hwp-transpiler-codec --bin hwp-to-md -- \
  --llm --editable --split-assets \
  --emit-roles --emit-editable --emit-domain-hints \
  doc.hwpx doc.md
```

- **편집하려면 반드시 `--editable`** — 기본(아카이브) 모드는 원본
  SECTION_BYTES를 얼려 두므로 md 편집이 복원에 반영되지 않는다.
- `--split-assets` — 그림을 `<stem>.assets.md` 로 분리해 본문 md를
  가볍게 유지. 복원 시 같은 stem이면 자동 페어링된다.
- `--edit-color=#RRGGBB` — AI가 고친 문단을 지정색으로 표시하고 싶을 때
  (`--editable` 자동 포함).
- 사람이 읽을 md가 필요하면 `--llm` 대신 `--emit-styles` 를 쓴다
  (이 출력은 복원용이 아니라 열람용).

## 2. 읽기

- 본문 `.md` 만 읽는다. `.assets.md` 는 base64 덩어리이므로 컨텍스트에
  올리지 말고 복원 때까지 보관만 한다.
- 구조: `SECTION[...]` / `PARAGRAPH[id=,level=]` / `TABLE[...]` /
  `CELL[...]` / `FIGURE[...]` 레코드 줄 + `TEXT:` 본문 줄.
  ID(`sec- par- tbl- cell- fig-`)는 위치 기반이라 재변환해도 안정적이다.
- `role=label|value|header`, `editable=true|false`, `kind=<domain>`
  태그로 어떤 셀이 수정 대상인지 판단한다.

## 3. 편집 (황금 규칙)

전체 규칙은 [docs/llm-edit-prompt.md](../../../docs/llm-edit-prompt.md)
— 편집 전에 반드시 읽는다. 핵심:

- **오직 "사람이 읽는 본문 텍스트"만 고친다**: `TEXT: ` 뒤,
  `TEXT[...]: ` 의 `]:` 뒤 내용만.
- 대괄호 `[...]` 로 시작하는 레코드 줄(SECTION/PARAGRAPH/TABLE/CELL/
  FIGURE, id·span·char_shape 속성)은 **절대 수정·삭제·재배열 금지**.
- `editable=false` 셀, 라벨(`role=label`) 셀은 건드리지 않는다.

## 4. 검증 후 복원 (md → hwpx)

편집 후 레코드 줄이 보존됐는지 먼저 검증한다:

```sh
diff <(grep -E '^(SECTION|PARAGRAPH|TABLE|END TABLE|CELL|FIGURE)\[' original.md) \
     <(grep -E '^(SECTION|PARAGRAPH|TABLE|END TABLE|CELL|FIGURE)\[' edited.md)
# 예상 밖의 차이가 나오면 편집을 거부하고 다시 시도.
# 허용되는 차이: 새 문단을 위해 통째로 복사한 레코드 줄,
# --edit-color 사용 시 "실제 편집한" 레코드의 char_shape= 값 변경.
```

통과하면 복원:

```sh
cargo run -p hwp-transpiler-codec --bin md-to-hwpx -- edited.md out.hwpx
# <stem>.assets.md 가 옆에 있으면 자동으로 페어링됨
```

복원 후 가능하면 `out.hwpx` 를 다시 `hwp-to-md --llm` 으로 뽑아
편집 의도가 반영됐는지 reparse-diff 로 확인한다.

## 주의

- `.hwp`(구형 바이너리)로 직접 시작하는 왕복은 PAGE_DEF 여백 미파싱으로
  픽셀 충실도가 떨어진다. `.hwp` 는 한컴에서 `.hwpx` 로 한 번 저장한 뒤
  `hwpx ↔ md` 경로를 쓴다.
- `md-to-hwpx` 에 `-` 를 출력으로 주면 바이너리가 stdout으로 흐른다 —
  터미널에 직접 흘리지 말 것.
