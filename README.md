# ts-hwp-transpiler

> 한글 공공문서를 조문·항·별표·서식 단위의 의미 구조로 해석한 뒤,
> 사람이 검토 가능한 HTML과 AI 처리 가능한 구조 표현을 동시에 생성하는
> 기술.

브라우저 네이티브 미리보기를 갖춘 **HWP/HWPX ↔ 마크다운** 양방향
트랜스파일러. 목표는 한국어 오피스 문서를 마크다운으로 라운드트립
시키되 구조 손실 없이 처리하고, 결과물을 일반 MD (LLM·에디터용) 또는
고충실도 미리보기 (사람용) 두 형태로 모두 렌더링하는 것.

> 상태: HWP5/HWPX 양쪽 reader/writer 동작 중, HWPX writer는
> DocInfo-side mutation도 surgical rewriter로 round-trip. WASM은
> handle-based 레지스트리로 큰 파일까지 처리하며 Vite 데모에서 라이브.
> 의미 구조 기반 HTML preview + Markdown export 모두 실문서 fixture로
> 검증됨. 원본 페이지의 픽셀 단위 fidelity 뷰는 `@rhwp/editor` iframe
> 임베드에 위임 — 자체 구현 안 함. 현재 진행 상황은
> `docs/memory/CURRENT.md` 참고.

## 이 프로젝트가 이미 보여준 가치

이 프로젝트는 아직 완성된 문서 편집기나 범용 변환기가 아닙니다. 하지만
한글(HWP) 문서를 Markdown으로 옮길 때 왜 항상 구조가 무너지는지에 대해,
꽤 실용적인 돌파구를 보여주고 있습니다.

기존의 많은 변환기는 문서를 "보기 좋게" 평탄화하는 데 집중합니다. 그
결과 표가 깨지고, 병합 셀이 사라지고, 그림과 캡션의 연결이 흐려지고,
무엇보다 나중에 다시 원래 문서 구조로 되돌리기 어려워집니다.

이 프로젝트는 반대로, 문서를 예쁘게 납작하게 만드는 대신 **구조를 잃지
않는 것**을 먼저 해결하려고 합니다. 그 결과 지금 단계에서도 이미 다음
같은 장점이 있습니다.

### 1. 문서의 "양식"을 잃지 않는다

국가 과제 계획서, 사업계획서, 공공기관 제출문서처럼 표와 셀 구조 자체가
문서의 의미인 경우가 많습니다.

이 프로젝트는 그런 문서를 단순 텍스트 덩어리로 바꾸지 않고, 표, 병합
셀, 셀 위치, 그림, 캡션 같은 구조를 최대한 유지한 채 Markdown/구조
텍스트로 옮기는 방향을 취합니다. 즉, "내용만 대충 추출"하는 도구가
아니라 **양식이 중요한 문서에 특히 강합니다**.

### 2. LLM이 읽기 좋은 문서 표현으로 확장 가능하다

이 프로젝트의 진짜 강점은 단순 변환이 아니라, 문서를 **LLM이 이해하고
수정할 수 있는 구조적 표현**으로 바꿀 수 있다는 점입니다.

예를 들어 문단, 표, 셀, 그림, 캡션에 식별자를 붙이고, 어떤 셀이 라벨인지
값인지, 수정 가능한지 아닌지를 드러내면 LLM은 문서를 훨씬 덜 헷갈리게
읽을 수 있습니다. 이건 특히 **"양식은 유지하고 텍스트만 수정"**해야
하는 문서 워크플로우에서 큰 장점입니다.

### 3. 사람이 아니라 "문서 구조" 중심으로 다룬다

보통 변환 결과는 사람이 읽기 좋게 만드는 데 치우치지만, 실제로는 사람이
조금 불편하더라도 문서 구조가 명확히 남는 표현이 더 유용한 경우가
많습니다.

이 프로젝트는 바로 그 지점을 노립니다. 즉, "예쁜 Markdown"보다 다시
처리하고, 비교하고, 재삽입할 수 있는 Markdown/중간표현을 우선합니다.
그래서 문서를 한 번 변환하고 버리는 게 아니라, **AI 보조 작성, diff,
슬롯 단위 수정, 다시 HWP로 복원**하는 흐름의 기반이 됩니다.

### 4. 기존 문서를 망가뜨리지 않는 방향으로 발전하고 있다

아직 모든 구조를 완전히 이해하지 못하더라도, 모르는 부분을 버리거나
뭉개지 않고 **보존한 채 다시 쓸 수 있는 전략**을 택하고 있습니다.

이 덕분에 기능을 하나씩 늘려도, 기존 문서를 깨뜨리지 않고 점진적으로
품질을 올릴 수 있습니다. 즉 이 프로젝트는 "완전히 해석할 수 있을 때까지
못 쓰는 도구"가 아니라, **안전하게 보존하면서 점점 더 똑똑해지는
엔진**에 가깝습니다.

### 5. 표가 많은 실무 문서에 특히 실용적이다

논문이나 자유 글보다, 오히려 표가 많고 양식이 강한 문서에서 더 큰
가치를 보여줍니다. 예를 들어:

- 국가 과제 계획서
- 사업계획서
- 연구개발 제안서
- 공공기관 제출 양식
- 예산표 / 성능지표표가 많은 문서

이런 문서는 일반 Markdown 변환기로는 금방 의미가 무너지는데, 이
프로젝트는 그 문서들을 **"LLM이 다룰 수 있는 구조화 문서"**로 바꾸는 데
실제로 가능성을 보여주고 있습니다.

## 실무 활용 가치

- **복잡한 한글 공문서의 의미 단위 보존**
  단순 텍스트 추출이 아니라 조문, 항목, 표, 별표, 서식 등 문서의 의미
  단위를 분리·구조화하여, 국가과제 공고·계획서·지침류 문서의 논리 구조를
  유지한 채 기계가 해석 가능한 형태로 변환할 수 있습니다.

- **질의응답형 탐색에 유리한 구조 표현**
  "누가", "무엇을", "언제까지", "어떤 조건에서"와 같은 실무형 질문에
  대해, 전체 문서를 다시 읽지 않고도 관련 블록을 직접 찾을 수 있는
  구조를 제공하므로, 일반 OCR 텍스트나 평문 Markdown보다 검색·질의
  응답 정확도를 높일 수 있습니다.

- **표·라벨·값 관계의 명시적 표현**
  국가과제 문서에서 자주 등장하는 라벨-값, 항목-세부내용, 구분-기준-비고
  형태의 표 구조를 단순 시각 배치가 아니라 의미 관계로 표현함으로써,
  LLM이나 규칙 기반 엔진이 값을 안정적으로 추출·비교·검증할 수 있습니다.

- **한글 문서의 시맨틱 기반 HTML 렌더링 가능성 확보**
  구조화된 중간 표현을 기반으로 사람이 읽기 쉬운 HTML 미리보기를
  생성하면서도, 원문 의미 구조를 최대한 유지할 수 있어, "기계가 읽기
  좋은 표현"과 "사람이 검토하기 좋은 표현"을 동시에 제공할 수 있습니다.

- **RAG 및 에이전트 워크플로우에 적합한 포맷**
  문서를 의미 단위별 청크로 나누고 각 블록에 안정적인 식별자와 역할
  정보를 부여할 수 있어, 한글 문서 전용 검색, 근거 인용, 조건 검토,
  체크리스트 생성, 자동 요약, 초안 작성 등 후속 AI 작업에 직접 활용하기
  용이합니다.

- **규정·공고·계획서와 같은 실무 문서 처리에 특화된 효과**
  일반적인 HWP/HWPX 변환기는 텍스트 덤프나 시각 복제에 치우치는 경우가
  많으나, 본 방식은 국가과제 계획서, 공고문, 제출서식처럼 구조가 복잡하고
  실무 질의가 빈번한 문서를 대상으로, 실제 업무 활용성이 높은 구조화
  결과를 제공합니다.

- **원문 보존성과 후처리 확장성의 양립**
  원문 레이아웃·계층·표현 요소를 최대한 유지하면서도, 필요 시 LLM
  친화 포맷, HTML, 검색 인덱스, 체크리스트, 폼 입력용 데이터 구조
  등으로 재가공할 수 있어, 단순 변환기를 넘어 문서 이해 및 자동화
  플랫폼의 기반 기술로 확장 가능합니다.

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

## 브라우저 데모

`ts/` 디렉토리에 Vite 기반 데모 페이지가 있음. 에디터/미리보기는
[`@rhwp/editor`](https://www.npmjs.com/package/@rhwp/editor) iframe
임베드 (메뉴/툴바/편집 모두 포함), Markdown 출력은 이 프로젝트의
`exportMarkdown`이 생성. 에디터 UI는 의도적으로 다시 만들지 않음.

```sh
curl -sSf https://rustwasm.github.io/wasm-pack/installer/init.sh | sh
cd ts
npm install
npm run build:wasm                # crates/wasm → ts/src/wasm/
npm run dev                       # http://localhost:5173
```

`.hwp` / `.hwpx` 파일을 드롭하면 왼쪽에 rhwp-studio 전체 에디터,
오른쪽에 Markdown이 나옴. 오른쪽 옵션 토글로 LLM 구조화 모드 /
도메인 힌트 / role·editable 태그 / 인라인 스타일을 즉석에서 전환.

## 현재 동작하는 것

- **HWP5 read/write** — `/FileHeader`, `/DocInfo`, `/BodyText/Section{N}`,
  `/BinData/*` 모두 타입드 read + verbatim 스트림 캐시 기반 round-trip.
  Mutate된 record는 re-encode를 트리거하고, 안 건드린 스트림은 byte-equal
  유지.
- **HWPX read/write** — ZIP+XML 컨테이너 분해, `Contents/section{N}.xml`
  IR 왕복, `BinData/image{N}.{ext}` 자동 promotion. `Contents/header.xml`
  은 **surgical rewriter**가 처리 — DocInfo IR mutation
  (`para_shapes` align, `char_shapes` height/textColor/strike/underline,
  `font_faces` 이름, `border_fills` solid color)이 출력에 반영되며,
  IR이 노출하지 않은 영역(스타일·numbering·lineSpacing·typeInfo Panose)
  은 verbatim 통과해 손실 없음.
- **DocInfo 레코드** 타입화: `DocumentProperties`, `IdMappings`,
  `FaceName ×7 슬롯`, `BorderFill`, `CharShape`, `ParaShape`, `Style`,
  `BinData`. 미타입 태그는 `raw_records`로 무손실 통과.
- **BodyText 레코드** 타입화: paragraph header / text / char-shape runs /
  line segments, `LIST_HEADER` (cell), `TABLE` + cells, gso →
  SHAPE_COMPONENT → SHAPE_COMPONENT_PICTURE 체인을 `ControlKind::Picture`
  로. HWPX `<hp:pic>` 도 동일 IR로 디코드.
- **임베디드 바이너리** — HWP5 `/BinData/*` (DEFLATE 자동 해제) +
  HWPX `BinData/*` 둘 다 `IrDocument.bin_data`로 promotion. MIME
  자동 결정. 레거시 BMP/TIFF는 미리보기용 JPEG 트랜스코딩.
- **마크다운 export** — 헤딩 감지(스타일 이름 + 숫자 prefix 폴백),
  colspan 그리드 확장, 단일행 박스 표 → 본문 변환, wrapper 표 unwrap,
  긴 셀 nested sub-bullet, 빈 셀 range 압축, 한컴 PUA 글머리표
  (`󰊱` 등) → 표준 `①..⑳` 정규화. 병합 셀은 `[r,c] span N×M:`
  annotation으로 무손실 보존.
- **수식 → LaTeX** — HWP equation script를 토크나이즈해 LaTeX로 출력.
  MD/HTML export 양쪽에 wired.
- **HTML preview (구조형)** — 위치 기반 stable IDs (`sec-`, `par-`,
  `tbl-`, `cell-`, `fig-`, `cap-`), `<section class="hwp-chapter
  hwp-lv-N">` 중첩 챕터, ParaShape align → `text-align`,
  `aspect-ratio` 보존, base64 data URI 인라인 그림. 픽셀 fidelity 뷰는
  rhwp iframe에 위임.
- **WASM + 데모** — handle-based doc registry (62MB+ 처리), rhwp-editor
  iframe lazy-load, HTML/MD 탭 전환, PDF/MD 다운로드.

TRL R&D 계획서 fixture (5 MB, 53 표, 9 임베디드 이미지) + 실 사업계획서
HWPX (3 MB, 그림 다수) 기준으로 round-trip + export 모두 통과.

## 아직 없는 것

- **MD → HWP/HWPX 진짜 양방향**의 마무리 — read/write IR은 동작하지만
  새 문서를 from-scratch로 쓸 때 일부 미타입 레코드 (`ID_MAPPINGS`,
  `NUMBERING`, `FACE_NAME emit`, `TRACK_CHANGE_*`, `LAYOUT_COMPATIBILITY`)
  는 hwplib 템플릿 번들에 의존. typed encoders로 점진 대체 중.
- **HWPX writer Phase 2** — `<hh:bold/>`/`<hh:italic/>` 토글
  (presence-only structural insert), 멀티스크립트 CharShape 배열
  (`<hh:fontRef>`, `<hh:ratio>` 등), paraPr/charPr 추가·제거, gradation
  /image fill mutation. Unmutated round-trip은 verbatim으로 통과되므로
  영향 없음.
- **이미지 마크다운 사이드카 dump** — IR 측 그림은 파싱되어 있고
  `MdOptions.assets_path` wiring 진행 중. CLI에서 `<doc>.assets/` dump
  미완.
- **각주·변경 이력 표면화** — verbatim 보존은 되지만 export 측에서
  의미 단위로 surface 안 됨.

## 자체 구현하지 않는 것

- **원본 페이지 픽셀 fidelity 렌더러** — `@rhwp/editor` iframe 임베드로
  대체. 데모의 첫 탭이 rhwp 뷰. 우리 HTML preview는 의미 구조 표현
  전용으로 유지.

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
