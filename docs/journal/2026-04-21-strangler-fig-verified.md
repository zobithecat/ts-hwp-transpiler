# 2026-04-21 — Strangler-fig pass-through: 실측 검증 완료

## Context

앞선 엔트리(`2026-04-21-scope-and-strangler-fig.md`)에서 선택한 전략이 이론상
성립하는지, 실제 HWP 파일에서도 성립하는지 확인이 필요했음. 사용자가
`test/` 에 두 개의 fixture를 직접 배치:

- `test/fixture.hwp` (22,528 B) — 한컴 오피스 "빈 문서" 저장 (Java 런타임
  없어서 hwplib 경로 우회)
- `test/260420-1. 연구개발계획서(서식)[TRL점프업 1단계]_대진대_수정_1430_fin.hwp`
  (5,144,576 B) — 실제 국가 R&D 계획서. 9개 임베디드 이미지 + 복잡한 표.

## Finding

**모든 스트림이 바이트 동일 수준 round-trip PASS.** 다이프 러너 결과:

```
test/fixture.hwp                                     9 streams, all =
test/260420-1. ...TRL점프업...대진대_수정_1430_fin   18 streams, all =
```

스트림 구성 (실측 HWP에서 확인된 고정 셋):

```
/\x05HwpSummaryInformation   OLE property set (\x05 prefix, HWP-specific)
/FileHeader                  256 B 고정 시그니처 + 버전
/DocInfo                     문서 메타 레코드 트리
/BodyText/Section{N}         각 섹션 paragraph 레코드
/BinData/BIN<NNNN>.{jpg,png} 이미지 블롭
/PrvText                     미리보기 텍스트 (1~수KB)
/PrvImage                    미리보기 BMP/PNG
/DocOptions/_LinkDoc         분산 저장 옵션
/Scripts/DefaultJScript      자바스크립트 자동실행
/Scripts/JScriptVersion      스크립트 엔진 버전
```

테스트 집합도 업데이트됨 (`crates/codec/tests/round_trip.rs`):
- `synthetic_streams_round_trip` — fixture 없어도 돎 (IR 합성)
- `blank_hwp_per_stream_match` — 빈 HWP fixture
- `trl_report_per_stream_match` — 실제 TRL 계획서

3/3 PASS in 0.10s.

## Why this matters

Strangler-fig 전략의 기본 가정 두 가지가 실측 검증됨:

1. **cfb crate + 자동 parent storage 생성**으로 OLE 컨테이너를 정상 재구성
   가능 — 스트림 콘텐츠 레벨에서 완벽한 복원.
2. **모든 HWP 파일은 `unknown_streams` BTreeMap 하나로 포용** 가능 —
   리더가 본 스트림 셋이 라이터 쪽에서 재구성 가능.

즉, **DocInfo writer 포팅 전에도 현재 코덱이 HWP read+write를 바이트 동일
수준으로 이미 지원함**. 단, 이 상태에서는 IR에 타입드 정보가 없어서
Markdown export 나 렌더러가 작동하지 않음 — 타입드 인코더가 추가될 때마다
`unknown_streams → typed IR` 이주가 일어나고, 이 테스트가 invariant guard.

**각 타입드 인코더 PR의 수락 기준은 동일**: "round-trip 테스트 3개 모두
초록 유지". 이것만 지키면 언제든 프로덕션 배포 가능한 상태를 보존.

## Consequence

- Fixture는 `test/*.hwp` 경로에 살고, `.gitignore`가 커밋 방지.
  `/test/.gitkeep` 으로 디렉토리만 추적.
- 테스트 경로는 `CARGO_MANIFEST_DIR + ../..` 로 workspace-rooted 해석.
- Semi-auto 러너를 실행하면 (0.1초 이내 완료) 개발자가 저장할 때마다
  회귀 즉시 포착.

## Open questions (carry-over + new)

- 5 MB TRL 문서를 매 `cargo test`마다 메모리에 올리는 비용 용인 가능 범위?
  향후 fixture가 수십 개로 늘어나면 `#[ignore]` + 전용 CI job 분리 필요.
- DocInfo 타입드 인코더 착수 시, **어떤 레코드부터** 옮길지 결정 필요.
  후보: FontFace (가장 단순) → BorderFill → CharShape → ParaShape → Style.
  실측 바이트 다이프는 항상 첫 divergence 를 알려주므로 순서는 실용적으로
  결정 가능.
- HWPX 쪽은 아직 0 % — blank.hwpx fixture가 필요해질 시점 미정.
