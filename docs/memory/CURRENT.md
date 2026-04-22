# 현재 위치

**지금**: Phase 2a-ii 완료. CLI sidecar dump + MD ↔ 파일 교차검증까지
green. 다음은 Phase 2b (caption 추출) 또는 L1 LLM skeleton.

**최근 ship**:
- `hwp-to-md` CLI 에 `--no-assets`, `--assets-dir=<path>`, `-h/--help`
  플래그 추가. 기본 동작: `doc.hwp` → `doc.md` + sibling `doc.assets/`
  디렉토리에 모든 `BinaryEntry` dump. stdout (`-`) 모드는 자동으로
  assets 를 비활성 (명시적 `--assets-dir` 로 override 가능).
- `crates/codec/src/export/assets.rs` 신설 — pure `dump_assets(doc, dir)`.
  Path traversal 가드 (id 에 `/`, `\`, `.`, NUL 포함 시 reject).
- Multi-paragraph 박스 제목이 `##` 로 승격되도록 `try_table_as_heading`
  relax. TRL fixture 7장 "연구개발성과의 활용방안 및 기대효과
  (기술성·시장성 및 사업성 검토 방안 등)" 이 본문으로 내려앉던
  이슈 수정 — cell.paragraphs.len() != 1 가드가 원인이었고, 이미
  존재하던 `try_table_as_passage` 의 space-join 패턴과 동일하게 처리.

**다음 후보** (병렬 가능):
- Phase 2b: caption 추출 — gso 컨트롤 안의 sub-paragraph 에서
  caption text 를 surface, `PictureControl` 과 연결 (`{{그림 N.}}`
  placeholder 가 `{{그림 N. <caption>}}` 이 되도록)
- L1 (LLM-friendly layer skeleton): `MdOptions` 에 `llm: Option<LlmOptions>`
  추가, section/paragraph/table/cell id 만 emit, role/editable 은 전부
  `unknown` 으로 시작. 기존 human 출력 불변 (opt-in).

**막힌 것**: 없음.

**작업 트리**: 깨끗. 테스트: 182/182 green (신규 27: assets 5 + CLI
17 + heading 3 + xref 2).

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
