# 현재 위치

**지금**: L1 (LLM-friendly layer skeleton) opt-in 가능. `MdOptions.llm =
Some(LlmOptions)` 또는 CLI `--llm` 으로 structured markdown 출력.
stable positional id 기반. role/editable 은 현재 placeholder (`unknown`).

**최근 ship**:
- `crates/codec/src/export/markdown_llm.rs` 신설. `to_llm_markdown(doc, opts)`
  가 SECTION / PARAGRAPH / TABLE / CELL / TEXT / FIGURE / CAPTION /
  END TABLE 마커로 구성된 record-style markdown 출력.
- **ID scheme**: purely positional path, 주소 결정적 & globally unique.
  - `sec-{si}`, `par-s{si}-p{pi}`
  - `tbl-s{si}-p{pi}-c{ci}` (controls index within owning paragraph)
  - `cell-<tbl_id>-r{r}c{c}`
  - 중첩시 경로 누적: `tbl-s0-p5-c0-r2c3-p1-c0`
  - `fig-{bin_id}`, `cap-fig-{bin_id}` (BinData 는 globally unique)
- `instance_id` 는 사용하지 않음 — 한컴 문서에서 대부분 0x80000000
  로 비유일하게 발견됨 (empirical on TRL). ID 안정성은 위치에서.
- TRL fixture 실측: 12 paragraph id / 1062 cell id 100% unique.
- CLI `--llm` 플래그 추가. Human / LLM 출력 상호배타.

**알려진 갭**:
- **role / editable 실제 분류기 미구현**. flag 을 켜도 전부 `unknown`
  만 emit. 이후 작업: bg_fill / bold / 위치 / 텍스트 패턴 휴리스틱
  기반 classifier. 보수적으로 unknown default.
- **Non-picture gso 의 caption**: 여전히 버려짐 (Phase 2b 원래 갭).
- **셀-임베드 picture MD**: human path 는 이번 세션에서 처리됨.
  L1 path 는 셀 안의 FIGURE 도 계속 emit 한다.

**다음 후보**:
- L2: cell role 휴리스틱 (bg_fill / alignment / 짧은 텍스트 / 첫 행
  행 / bold). 보수적으로 시작.
- L3: editable 추정 (role=value + 단일 paragraph + 수식/숫자 아님).
- L4: figure/caption 전역 domain hint (performance_metrics, budget 등).
- Preview layer (render crate) — IR → HTML with rowspan/colspan.

**막힌 것**: 없음.

**작업 트리**: 깨끗. 테스트: 199/199 green.

**빠른 컨텍스트**:
- 라운드트립이 1순위 목표; 마크다운 품질은 2순위. 명시적 결정은
  `docs/journal/2026-04-22-rowspan-marker-reverted.md` 의 "시각적 이득
  위해 마크다운에서 lossy 변환 금지" 항목.
- 스펙 사실은 이 디렉토리의 `hwp5-spec-notes.md` 에.
- hwplib 포팅 맵은 이 디렉토리의 `hwplib-mapping.md` 에.
- 저널 엔트리 (`docs/journal/`) 는 *언제·왜* 를 기록; 여기 문서는
  *지금 무엇이 사실인가* 를 기록.

stale해지면 (HEAD가 이동하거나 테스트가 변하거나 막힌 것 생기면)
업데이트할 것 — 그게 이 문서의 존재 이유.
