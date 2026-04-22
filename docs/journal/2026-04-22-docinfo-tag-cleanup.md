# 2026-04-22 — DocInfo 태그 식별 (Round 3 minimum)

**맥락**: 어제 `2026-04-22-rounds-1-2.md` 가 DocInfo `raw_records` 에서
알 수 없는 태그 두 개를 Open 질문으로 남겼음: `0x0020`, `0x005E`. 세션-3
계획은 착수 전에 답이 필요했음.

**발견됨** (hwplib `object/etc/HWPTag.java` 에서):

- `0x0020` = **TRACK_CHANGE_INFO** (변경 추적 정보) — 변경 추적 메타데이터
  블록.
- `0x005E` = **FORBIDDEN_CHAR** (금칙처리 문자) — 한국어 줄바꿈 금칙 문자
  설정.

둘 다 유효한 HWP5 스펙 태그이지만 niche. 바이너리 레이아웃을 공개 스펙에서
찾을 수 없었음 — hwplib에 stub reader는 있지만 field semantics 없음.

`HWPTag.java` 를 보면서 미선언 DocInfo 태그 세 개 추가 발견: `0x005C`
MEMO_SHAPE, `0x0060` TRACK_CHANGE (body), `0x0061` TRACK_CHANGE_AUTHOR.
아직 우리 fixture에서는 나타나지 않았지만 스펙의 일부.

**결정**: `streams::doc_info::tag` 에 다섯 개 태그 상수를 추가하여 코드가
숫자 hex 대신 이름으로 참조 가능하게. **typed parser는 추가하지 않음** —
`raw_records` 가 이미 verbatim으로 보존하고, strangler-fig writer는
`stream_bytes` 로 라운드트립.

**이유**: 다섯 개 태그가 두 그룹으로 분류됨. Track-changes 레코드 (0x0020
/ 0x0060 / 0x0061) 는 변경 추적 상태를 담는데 transpiler가 충실하게
라운드트립하려면 해석 불필요 — 게다가 잘못 다루면 한컴의 서명·무결성 검사를
깰 수 있음. Forbidden-char와 memo-shape도 마찬가지로 niche. Verbatim
passthrough가 추측된 레이아웃보다 더 안전한 기본값.

**결과**: track-changes 파서를 나중에 추가하려면 변경 추적이 실제로 포함된
fixture 필요 — TRL 계획서는 그걸 포함하지 않음.

**미해결**: BinData (`0x0012`) 가 여전히 가치 대비 가장 높은 미타입 DocInfo
레코드. 이미지 참조를 담고 있어서 마크다운 exporter가 현재 `<img>` 참조를
surface할 수 없음. wiring 은 (a) BinData record body 타입드 parser, (b)
`/BinData/*` 스트림을 `unknown_streams` 에서 typed `binary_files` 맵으로
이동, (c) body에서 `PictureControl` 과의 cross-referencing을 요구.
다음 세션 크기의 chunk로 예상.
