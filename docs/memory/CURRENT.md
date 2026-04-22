# 현재 위치

**지금**: Preview (render crate HTML) 1차 구현. `hwp_transpiler_render::
to_html(doc)` 가 `<article class="hwp-preview">` fragment 를 반환.
표는 `<table>` + `rowspan`/`colspan` 속성으로 복원, 그림은 `<figure>
<img><figcaption>`, heading 은 outline style 기반.

**최근 ship**:
- `crates/render/src/html.rs` 신설. `to_html / to_html_with +
  HtmlOptions { assets_path }`. render crate 의 기존 RenderCommand
  기반 pixel pipeline 과 독립된 경로 (page layout 은 아직 scaffold
  그대로).
- Markdown 과 HTML 책임 분리 원칙 준수:
  - Markdown: lossy-safe, 구조 보존, LLM 친화
  - HTML preview: 시각적 복원 (rowspan/colspan), 사람 친화
- 텍스트 cleanup (FFFC/NBSP/em-space/PUA circled digits),
  HTML 엔티티 이스케이프 (`& < > " '`), `strip_caption_label_prefix`
  모두 render 쪽에 재구현 (codec 역방향 의존 방지).
- TRL fixture smoke test: 53 tables / 9 figures 모두 HTML 구조로
  surface, rowspan/colspan 포함, FFFC leak 없음, `&` 엔티티
  이스케이프 검증.

**알려진 갭**:
- **CharShape 기반 폰트/색상 렌더링 미구현**. 텍스트는 plain.
- **Page / column 경계 미구현**. 전체 document 가 하나의 fragment.
- **Equation 렌더링 미구현** — controls 는 emit 단계에서 drop.
- **CSS 포함 안 됨** — 호출자가 자기 스타일 적용 필요.
- **Box-as-heading 는 preview 에서 decode 안 함** — Markdown path
  만 처리. 호환성 위해 둘을 나중에 통합할지 결정 필요.

**다음 후보** (리스트 순서대로):
1. ~~Preview~~ ✅
2. **Non-picture gso caption** — line/rect/ole shape 의 caption 살리기
3. **Fixture 확대** — TRL 외 heading-heavy / 수식-heavy / tracked-change
4. **Schedule heuristic** — gantt-style 표 구조 감지
5. **Figure domain hint**

**막힌 것**: 없음.

**작업 트리**: 깨끗. 테스트: 242/242 green.

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
