# 현재 위치

**지금**: `caption_text` 필드가 `PictureControl` → `Control` 으로 승격.
이제 picture 뿐 아니라 line / rectangle / OLE / 기타 non-picture gso
도 caption 을 IR 에 surface 할 수 있음. 기존 picture caption 경로는
그대로 유지.

**최근 ship**:
- IR 변경:
  - `Control { kind, caption_text: Option<String> }` (신규 필드)
  - `PictureControl.caption_text` 제거
  - `ControlKind::Default = Unknown { ... }` (Control::Default 지원)
- Parser (`body_text::parse_paragraph`): gso 종료 시
  - `$pic` 으로 판명되면 picture + Control.caption_text
  - 다른 shape 이면 Unknown 유지 + Control.caption_text 세트 (이전엔
    drop 되었음)
- Exporter 마이그레이션:
  - `markdown::emit_picture / emit_picture_bullet` 가 caption_text
    를 별도 인자로 받도록 변경
  - `markdown_llm::emit_figure` 동일
  - `render::html::emit_figure` 동일
- 모든 테스트 construct 사이트 마이그레이션 (Control 리터럴 ~20곳).

**검증**:
- TRL fixture: 9 picture → 6 caption surface (이전과 동일)
- TRL fixture: 0 non-picture gso caption (이 fixture 에 해당 조합
  없음 — 인프라는 완성)
- 243 tests green, 라운드트립 byte-equal 유지.

**알려진 갭**:
- Non-picture gso caption 동작은 synthetic test 또는 다른 fixture
  확보 필요.
- 수식/OLE 등 non-picture gso 자체의 typed IR 는 아직 Unknown.

**다음 후보** (리스트 남은 것):
3. **Fixture 확대** — TRL 외 heading-heavy / 수식-heavy / tracked-change
4. **Schedule heuristic** — gantt-style 표 구조 감지
5. **Figure domain hint**

**막힌 것**: 없음.

**작업 트리**: 깨끗. 테스트: 243/243 green.

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
