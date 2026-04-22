# Fixture corpus

라운드트립 하네스 (`tests/round_trip.rs`, binary `hwp-roundtrip`) 가
**hwplib** (canonical HWP binary writer, Java) 가 생성한 실제 HWP 파일
대비 우리 codec의 출력을 diff.

## hwplib에서 vendor된 파일

이 파일들은 [neolord0/hwplib](https://github.com/neolord0/hwplib)
저장소의 `sample_hwp/` 디렉토리에서 verbatim vendor (Apache License 2.0).
재-fetch:

```sh
curl -sL https://raw.githubusercontent.com/neolord0/hwplib/main/sample_hwp/blank.hwp        -o blank.hwp
curl -sL https://raw.githubusercontent.com/neolord0/hwplib/main/sample_hwp/merging-cell.hwp -o merging-cell.hwp
```

| 파일              | bytes  | 무엇을 테스트하나                                  |
|-------------------|--------|----------------------------------------------------|
| `blank.hwp`       | 22,528 | `BlankFileMaker.make()` baseline — 빈 문서          |
| `merging-cell.hwp`| 10,752 | row_span / col_span 셀 LIST_HEADER 파싱             |

새 코드 path가 활성화되면 `sample_hwp/` 에서 더 추가 (image, equation,
distribution 등).

## Runner

```sh
cargo run -p hwp-transpiler-codec --bin hwp-roundtrip -- \
  crates/codec/tests/fixtures/blank.hwp
```

Semi-auto watch ([`watchexec`](https://github.com/watchexec/watchexec)
또는 [`cargo-watch`](https://github.com/watchexec/cargo-watch) 먼저 설치):

```sh
./scripts/watch-roundtrip.sh crates/codec/tests/fixtures/blank.hwp
```
