# ts-hwp-transpiler

브라우저에서 돌아가는 **HWP / HWPX ↔ Markdown 양방향 변환기**.
한글 공공문서(국가과제 계획서·사업계획서·공고문 같은 표 많은 양식
문서)를 구조 손실 없이 마크다운으로 옮기고, 다시 HWPX로 복원합니다.
서버 업로드 없이 전부 브라우저(WASM)에서 처리됩니다.

> 🔗 **라이브 데모:** <https://ts-hwp-transpiler.vercel.app>
>
> `.hwp` / `.hwpx` / `.md` 파일을 끌어 놓기만 하면 됩니다.

---

## 데모 페이지로 할 수 있는 것

### 1. 미리보기 3종

파일을 올리면 좌측 패널에서 세 가지 뷰를 탭으로 전환할 수 있습니다.

| 탭 | 어떤 뷰 |
|---|---|
| **HTML** | 이 프로젝트의 자체 구조형 렌더러. 의미 단위(섹션·문단·표·셀·그림·캡션)별 stable ID + ParaShape align 반영. |
| **에디터** | [`@rhwp/editor`](https://www.npmjs.com/package/@rhwp/editor) iframe — 한컴 그대로의 픽셀 fidelity 페이지 렌더링 + 편집 UI. |
| **Markdown** | 우측 옵션을 그대로 반영한 라이브 마크다운 뷰. 다운로드 직전에 어떤 결과가 나올지 미리 볼 수 있음. |

### 2. Markdown 출력 (구조 보존)

우측 카드에서 출력 옵션을 토글하면 좌측 Markdown 뷰가 즉시 반영됩니다.

- **LLM 구조화 모드** — `SECTION[id=...]` / `PARAGRAPH[id=...,level=N]` / `TABLE[...]` / `CELL[...]` / `FIGURE[...]` 레코드 표기.
  같은 문서를 LLM에 두 번 보내도 ID가 안정적이라 diff/슬롯 수정에 유리.
- **role · editable 태그** — 어떤 셀이 라벨인지 값인지, 수정 가능한 셀인지 명시.
- **도메인 힌트** — 기관 정보 / 예산표 / 일정표 등 자주 등장하는 표 유형 자동 라벨.
- **인라인 스타일** — `**bold**` · `*italic*` · `~~strike~~` (꺼면 평문 텍스트).
- **그림 처리** — 3 모드:
  - `텍스트만` (그림 자리에 `[FIGURE]` 마커만)
  - `인라인 base64` (단일 자족 `.md` 파일)
  - `분리` (`doc.md` + `doc.assets.md` 페어 — LLM 컨텍스트 절약)
- **DPI 선택** — 72 (기본) / 36 (LLM 컨텍스트 절약용 절반 사이즈)

### 3. HWPX 다운로드 (passthrough)

지금 메모리에 있는 IR을 그대로 `.hwpx`로 저장. 원본의 그림과 메타가
손실 없이 보존됩니다. 라운드트립이 필요하면 .md 다운로드 → 다시
업로드 사이클을 쓰면 됩니다.

### 4. HTML / PDF 다운로드

구조형 HTML 미리보기를 정적 HTML 파일 또는 인쇄용 PDF로 내보냅니다.

### 5. .md ↔ .hwpx 라운드트립

`.md` 파일을 업로드하면 그대로 IR로 파싱되어 HWPX로 다시 저장
가능합니다. 분리 모드로 내보낸 `.md` + `.assets.md` 페어를 한 번에
멀티 셀렉트해서 올리면 자동 페어링됩니다.

---

## 왜 이걸 만들었나

기존 HWP→마크다운 변환기 대부분은 문서를 "보기 좋게" 평탄화하는 데
집중합니다. 그 결과 표가 깨지고, 병합 셀이 사라지고, 그림과 캡션
연결이 흐려지고, 무엇보다 **다시 HWP로 되돌리기 어려워집니다**.

이 프로젝트는 반대 방향입니다. 예쁘게 납작하게 만드는 대신,

- **구조를 잃지 않는 것**을 먼저 해결합니다 (표·병합 셀·셀 위치·그림·캡션 유지)
- **LLM이 다루기 좋은 표현**을 출력합니다 (stable ID, role/editable, 도메인 힌트)
- **다시 HWPX로 복원**이 가능한 마크다운을 정의합니다

그래서 단순 추출 도구가 아니라, **AI 보조 작성 / diff / 슬롯 단위
수정 / HWP 복원** 흐름의 기반 엔진을 목표로 합니다.

---

## 동작 방식

```
브라우저
  └─ Vite + TypeScript demo (ts/)
       ├─ @rhwp/editor (iframe) ── 픽셀 fidelity 렌더링
       └─ hwp_transpiler_wasm   ── 우리 변환 엔진
            └─ Rust 워크스페이스 (crates/)
                 ├─ core/   IrDocument + Reader/Writer trait
                 ├─ codec/  HWP5/HWPX reader·writer + MD exporter (대부분의 로직)
                 ├─ render/ 백엔드 무관 RenderCommand
                 └─ wasm/   wasm-bindgen surface
```

핵심 설계:

- **handle-based registry** — 62MB+ 파일을 메모리 효율적으로 처리.
- **surgical rewriter** — HWPX `header.xml`의 DocInfo IR mutation
  부분만 재작성, 나머지는 verbatim 통과 → **HWPX 라운드트립 컨테이너
  byte-equal** 13개 stream 달성.
- **verbatim 스트림 캐시** — HWP5 OLE 스트림 중 IR이 모르는 영역은
  원본 바이트 그대로 보존, 건드린 레코드만 re-encode.

---

## 로컬 실행

```sh
# 1. wasm-pack 설치
curl -sSf https://rustwasm.github.io/wasm-pack/installer/init.sh | sh

# 2. 데모 페이지 실행
cd ts
npm install
npm run build:wasm   # crates/wasm → ts/src/wasm/
npm run dev          # http://localhost:5173
```

CLI도 있습니다 (`cargo run -p hwp-transpiler-codec --bin hwp-to-md -- doc.hwp`),
자세한 사용법은 [`docs/PROJECT-HISTORY.md`](docs/PROJECT-HISTORY.md) 참고.

---

## 현재 한계

- **HWP5 → MD → HWPX layout 정확도** — HWPX 원본 라운드트립은
  byte-equal 까지 도달했지만, HWP5(.hwp) 입력은 MD 포맷이 doc_info
  (charShape/paraShape/style)를 encode 하지 않아 round-trip 시 모든
  paragraph가 default 스타일로 떨어집니다. 그림은 정상.
- **lossy 이미지 인코딩** — 안전성을 위해 의도적 미지원. 파일 크기는
  `DPI=36`으로 절반 가능.
- **각주·변경 이력** — verbatim 보존은 되지만 export에서 의미 단위로
  surface 안 됨.

---

## 더 깊이

- **엔지니어링 히스토리·기술 상세**: [`docs/PROJECT-HISTORY.md`](docs/PROJECT-HISTORY.md)
- **현재 상태 (live reference)**: `docs/memory/CURRENT.md`
- **HWP5 바이너리 포맷 노트**: `docs/memory/hwp5-spec-notes.md`
- **설계 결정 저널**: `docs/journal/`

---

## 라이선스

미정 (TBD).
