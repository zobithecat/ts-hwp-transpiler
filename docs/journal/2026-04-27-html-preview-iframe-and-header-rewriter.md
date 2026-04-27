# 2026-04-27 — HTML preview 스코프 + HWPX header.xml surgical rewriter

## 맥락

04-22 이후로 약 4일간 데모 / WASM / HTML preview / HWPX 측에 다수의
변경이 누적됐고, 이번 세션에서 그 흐름의 마지막 두 결정을 내림.
저널은 결정·이유 위주이므로 04-23~04-26 호흡은 git log에 위임하고
이 엔트리에는 *오늘 내린* 두 결정만 영구 기록.

## 결정 1 — HTML preview는 structural-only, 픽셀 fidelity는 rhwp iframe에 위임

이전 `project_scope` 약속(2026-04-21)에는 "원본 페이지 레이아웃을
픽셀 수준으로 재현하는 고충실도 렌더러"가 5번 항목으로 있었음. 오늘
이를 **delegated** 상태로 변경.

**결정**: `crates/render/src/html.rs`는 의미 구조 (heading, table,
figure, alignment, indentation, IDs) 표현에 집중. 페이지 단위 픽셀
재현은 추구하지 않음. 데모는 이미 `@rhwp/editor` iframe을 첫 탭으로
싣고 있으므로 사용자는 fidelity 뷰를 그쪽에서 봄(`a7b2ed4` 시점에 default
탭 = HTML preview, lazy iframe).

**이유**:
- rhwp가 이미 풀 fidelity 뷰어 + 편집 UI까지 제공. SVG/Canvas/CSS-grid
  기반 자체 fidelity 모드는 같은 가치를 두 번 만드는 일.
- 우리 차별화 포인트는 **양방향 변환 + binary HWP write + 구조화 IR**.
  fidelity는 보조 가치이고, rhwp 임베드로 충분.
- "structural HTML preview"는 LLM 친화 표현 / 본문 검토 / 마크다운과
  나란히 비교하는 용도에 더 어울림. 페이지 fidelity와 표현 목표가 애초에
  다름.

**기각된 대안**: 자체 SVG/Canvas fidelity 모드 — 비용 대비 가치 낮음으로
판단.

**결과**:
- `project_scope` memory의 #5는 의미 변경 없이 "fidelity는 외부 위임"
  으로 해석.
- 패텐트/가치 제안 문서에서 "고충실도 렌더러"를 자체 기능으로 광고하지
  않을 것.
- HTML preview에 fidelity 요구 (좌표 기반 layout, 절대 위치 등) 끌어들이지
  않음.

## 결정 2 — HWPX `Contents/header.xml`은 surgical rewriter로

writer가 지금까지 `unknown_streams`에서 verbatim으로 흘려보내고 있어
DocInfo-side mutation (`font_faces`, `char_shapes`, `border_fills`,
`para_shapes`)이 출력에 반영되지 않는 한계가 있었음. 두 가지 선택지를
검토 후 **surgical rewriter** 방식 채택.

**기각된 대안 — 풀 재생성**: IR → header.xml을 처음부터 다시 emit.
간단하지만 `parse_header_xml`이 디코드하지 않은 영역(`<hh:styles>`,
`<hh:numberings>`, `<hh:bullets>`, `<hh:tabProperties>`, `<hh:lineSpacing>`,
`<hh:border>`, `<hh:typeInfo>` Panose 데이터, `<hh:substFont>`, kerning
플래그 등)이 모두 손실. 문서 의미 변형 — 받아들일 수 없음.

**채택 — surgical rewriter**: 원본 XML을 `quick-xml`의 byte cursor 위에서
streaming하면서, IR이 노출하는 attribute만 IR값으로 갈아끼우고 나머지
바이트는 verbatim 통과. 메커니즘:

1. `reader.buffer_position()`으로 각 이벤트의 `[start, end)` 바이트 범위를
   안다.
2. 관심 이벤트가 오면 `original[cursor..event_start]` 를 통과로 emit.
3. 그 자리에 IR-derived attribute로 재구성된 태그를 emit.
4. `cursor = event_end`로 점프, 원본 이벤트 바이트는 건너뜀.
5. 미관심 이벤트는 cursor 그대로 — 다음 통과 시 verbatim 포함.
6. EOF에서 `original[cursor..]` 잔여 flush.

attribute 재구성 시 이미 존재하는 attribute들의 raw bytes는 그대로
복사 (entity reference `&amp;` 등 보존), override만 escape.

**Phase 1 구현 범위**:
- `<hh:align horizontal=…>` (paraPr 자식)
- `<hh:charPr height/textColor/shadeColor/borderFillIDRef>` 부모 attr
- `<hh:strikeout shape=… color=…>`, `<hh:underline shape=… color=…>` 자식
- `<hh:font face=…>` (fontface 자식)
- `<hc:winBrush faceColor=…>` (borderFill solid color만)

**Phase 2 의도적 보류**: bold/italic 토글(presence-only이라 structural
insert/skip 패턴 필요), 멀티스크립트 CharShape 배열(`<hh:fontRef>`,
`<hh:ratio>`, `<hh:relSz>`, `<hh:spacing>`, `<hh:offset>`), 새 shape
추가/제거(structural), gradation/image fill mutation(IR 측 선행
enrichment 필요).

**결과**:
- DocInfo mutation flow가 끊김 없이 동작 — `paraShape.attribute &= !0x07; |= 0x02`
  같은 IR 측 편집이 출력 HWPX에 그대로 반영되어 재읽기 시 살아남음.
- Unmutated round-trip은 verbatim과 의미적으로 동등 — 통합 테스트
  `real_hwpx_unmutated_round_trip_preserves_doc_info_shapes`가 보장.
- 다음 단계는 Phase 2 — bold/italic / 멀티스크립트 / shape add-remove /
  fill 타입화 + 임시 mutation. IR 측 변경이 필요한 #6 fill 타입화는
  가장 무거운 항목.

## 미해결 질문

- Phase 2 진행 시 `Fill` IR을 어떻게 enrich할지 (typed `gradation:
  Option<Gradation>`, `image: Option<FillImage>` 추가 vs `body:
  Vec<u8>`에 typed parser 두 단 wrap). 후자가 round-trip 안전.
- `<hh:font face>` 변경 시 `<hh:substFont>` 자식이 stale해질 수 있는데,
  현재 rewriter는 substFont를 건드리지 않음. font name 바꾸는 사용자가
  대체 폰트도 같이 바꿔야 한다는 의미 — 문서화 필요.
