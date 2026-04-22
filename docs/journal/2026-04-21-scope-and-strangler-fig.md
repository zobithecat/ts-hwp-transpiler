# 2026-04-21 — 스코프 확정 + strangler-fig 라운드트립 전략

## 맥락

`task.md` 한 장 외에 아무것도 없는 상태에서 시작. 첫 턴에는 HWP → MD
단방향 추출기 전제로 스캐폴드(core: semantics/hint/formula, 7 tests pass).
이후 사용자 요구가 단계적으로 확장됨:

1. "웹 브라우저에서 네이티브로 HWP/HWPX 미리보기 + MD 다운로드 + 역방향 가능"
2. "진짜 .hwp까지 써야 하고 원본 페이지 레이아웃 재현까지 필요"
3. hwplib 참조 지시 (https://github.com/neolord0/hwplib)
4. "B로 가즈아" — fixture-first 바이트 동일성 전략 확정
5. "반-자동 러너 고" — 다이프 러너 binary 추가

## 결정

**Crate 구성 (최종)**

```
crates/core       symmetric IR + semantics + hint + formula
crates/codec      HWP/HWPX Reader+Writer (구: writer, 리네임)
crates/render     backend-agnostic RenderCommand + layout
crates/wasm       wasm-bindgen glue
```

**Strangler-fig 라운드트립 전략**

- `HwpReader`는 OLE 스트림을 그대로 `IrDocument::unknown_streams`
  BTreeMap에 덤프
- `HwpWriter`는 `unknown_streams`를 그대로 OLE 스트림으로 재기록
- 1일 차부터 스트림-콘텐츠 수준 바이트 동일성 성립 (컨테이너 레이아웃은
  cfb의 allocator가 결정하므로 raw-byte 레벨 동일성은 보장하지 않음 —
  다이프는 스트림-by-스트림으로 수행)
- 타입드 인코더가 추가될 때마다 해당 스트림이 `unknown_streams`에서
  typed IR 필드로 이주하고, 라운드트립 테스트는 계속 green 유지가 조건

**Byte-identity reference stack**

- OLE 컨테이너: `cfb` crate (mdsteele/rust-cfb, pure Rust, 검증 완료)
- HWP5 레코드 인코딩: neolord0/hwplib (Java) 이 canonical — 포팅 대상
- rhwp(edwardkim)의 serializer는 HWPX만, HWPX-source save는 v0.7.3에서
  disable됨 → binary HWP writer는 rhwp 참조 불가

## 이유

alternatives considered:

- **(a) 포트 스캐폴드 쫙 깔고 점진 채우기** — hwplib의 ~10개 writer 패키지
  Rust 파일만 만들어두고 뒤에서 구현. Risk: 바이트 차이가 나도
  fixture 없이는 못 찾음. 구현 후반부 디버깅 지옥.
- **(b) Fixture-first strangler-fig** ✅ — hwplib으로 먼저 최소 HWP
  (BlankFileMaker) 생성 → 러너가 Reader+Writer 통과시키고 첫 divergence
  오프셋을 hex dump로 보여줌 → 타입드 인코더 추가하면서 라운드트립
  green 유지. 각 인코더 PR이 독립 검증 가능.

HWPX 우회 save 옵션은 사용자가 명시적으로 거부
(`memory/feedback_no_shortcuts.md`).

## 결과

- **task #10 (DocInfo writer 포팅)이 모든 binary HWP 경로의 게이트**.
  진입 전에 fixture `blank.hwp` 필요 — `crates/codec/tests/fixtures/README.md`에 hwplib `BlankFileMaker` 사용법 문서화됨.
- 다이프 러너 binary: `cargo run -p hwp-transpiler-codec --bin hwp-roundtrip -- <fixture>`. 스트림별 길이 요약 + 첫 divergence 오프셋 + 양쪽 16-byte hex context 출력.
- Semi-auto watch: `./scripts/watch-roundtrip.sh` (watchexec/cargo-watch
  자동 감지)
- Rust 1.85+ 필요 (cfb transitive dep uuid v1.23.1 요건).
- IR의 `unknown_streams`와 `UnknownRecord`는 영구적 round-trip 장치 —
  신규 레코드 타입 발견 시 여기를 통과시키면 바이트 손실 없음.

## 검증됨

- `cargo build --workspace --exclude hwp-transpiler-wasm` green
- core 기존 7 tests pass (formula tokenizer + semantic grid/visual)
- codec synthetic_streams_round_trip PASS — `/FileHeader`, `/DocInfo`,
  `/BodyText/Section0` 3개 stream을 `unknown_streams`에 넣고 write→read
  했을 때 바이트 동일 복원 확인
- 첫 실행에서 `/BodyText/Section0` 작성이 실패 (cfb는 parent storage가
  명시적으로 존재해야 함). `HwpWriter::write()`가 `create_storage_all`로
  부모 storage 자동 생성하도록 수정. 이 제약은 OLE 스펙 차원의 것이라
  타입드 인코더에도 그대로 적용됨 — 경로 분리 유틸 `parent_storage_of`
  가 writer.rs에 살아있음.
- `blank_hwp_per_stream_match`는 fixture 부재로 skip (예상 경로)

## 미해결 질문

- **Phase 2 hwplib DocInfo 포팅 단위** — 한 레코드 타입씩(예: CharShape
  먼저) 가는 게 좋을지, 한 스트림 전체(DocInfo) 한번에 가는 게 좋을지.
  fixture 다이프 결과가 이걸 결정해줌 — 어느 레코드가 제일 먼저
  divergence를 일으키는지 보고 착수.
- **HWPX 쓰기 backend** — `zip` crate는 wasm32에서 컴파일은 되지만
  동작 검증 필요. 대안으로 `async_zip` 또는 수동 DEFLATE.
- **폰트 fallback 소스** — 함초롬바탕 등 시스템 폰트 없을 때 대체
  매핑이 어디서 올지 아직 미정. 브라우저 프리뷰 전에 결정 필요.
- **인코딩 검증 corpus** — `blank.hwp` 외 어떤 fixture를 골든 기준으로
  삼을지. 한컴 공식 샘플? 사용자 제공? 저장소 크기 고려 필요.
