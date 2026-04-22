# 2026-04-22 — cell LIST_HEADER 버그 수정 + 마크다운 export 품질 라운드

**맥락**: scaffold 이후 첫 실문서 (TRL R&D 계획서) 변환 결과 검수. 어떤
유닛 테스트도 잡지 못한 두 가지 대형 결함 발견: 모든 표 셀 좌표가 깨지고
(`[65024,11] span 36097×65025`), 수정 후에는 실제 표 대부분이 bullet 리스트로
덤프되거나 6KB짜리 한 줄로 평탄화됨. 두 종류의 버그를 이번 세션에서 모두
처리.

## 검증됨 — hwplib `ListHeaderForCell` 바이트 레이아웃

`streams/list_header.rs::parse_cell` 에 두 가지 잘못된 가정 존재:

- `paraCount` 을 `u16` 으로 가정 → 실제는 `sInt4` (preamble이 21이 아니라 8 bytes)
- `textWidth (uInt4)` 필드 누락 → fixed 영역이 26이 아니라 38 bytes

hwplib `reader/.../tbl/ForCell.java::listHeader` 소스 읽기로 실제 레이아웃
확인:

```
sInt4  paraCount        (4)
uInt4  property         (4)
uInt2  colIndex / rowIndex / colSpan / rowSpan          (4 × 2 = 8)
uInt4  width / height                                   (2 × 4 = 8)
uInt2  leftMargin / rightMargin / topMargin / bottomMargin  (8)
uInt2  borderFillId     (2)
uInt4  textWidth        (4)
                                                  (38 bytes 고정)
[opt]  uInt1 fieldNameFlag (0xff → ParameterSet) + 8-byte zero pad
```

끝에서 자르는 전략이 unsafe한 이유: hwplib writer는 항상 `flag(1B) + 8B zero
pad` trailing을 붙임 (fieldName 없을 때 47 bytes 총). offset-from-start로
재작성.

**결과**: `TableCell` IR에 `para_count`, `property`, `text_width_hwpu` 필드
추가. `parses_47_byte_cell_with_trailer` 회귀 테스트로 trailer 내성 고정.

## 결정 — 마크다운 export 표 분류 휴리스틱

`emit_table` 에서 top-down 순서로 확인:

1. `try_unwrap_wrapper_table` — 본문 텍스트 없이 nested table 하나만 있는
   1×1. wrapper를 strip하고 inner를 동일 depth에서 재귀.
2. `try_table_as_heading` — 비어있지 않은 셀 1개, 짧은 한 줄,
   `<숫자>. ` prefix (→ `##`) 또는 `(...) ` prefix (→ `###`).
3. `try_table_as_passage` (top-level 전용) — 단일 행, 비어있지 않은 셀
   1개, space-join 후 ≤ 100자, controls 없음. 일반 산문으로 emit.
4. `try_build_md_grid` — 모든 `row_span == 1`; 셀이 (col_span 확장 후)
   겹침·구멍 없이 grid를 채움; nested table 없음. MD grid로 emit.
5. 그 외 → `emit_table_as_list`. bullet 경로에서:
   - 같은 row의 unspanned 빈 셀 연속은 `[r,c1..c2]: (empty)` 로 압축;
   - join 결과 ≤ 200자면 ` · ` 인라인, 초과하면 paragraph당 `  - …`
     sub-bullet.

세 개의 숫자 임계값 (80 / 100 / 200 chars) 은 이 fixture 하나 기준으로 조정.

**이유**: 보수적-후 허용 방향. 이른 exit (unwrap / heading / passage) 는 표
추상화를 완전히 버리므로 조건이 가장 엄격. grid 경로는 GFM이 표현 가능할 때
table-ness를 유지. bullet 경로는 데이터 손실 없이 나머지를 처리 — 빈 범위
압축과 길이 인식 inline/explode 쌍은 이전 형태 (모든 빈 셀 별도 줄, 또는 모든
내러티브를 ` · ` 한 줄로 평탄화) 의 노이즈 문제를 없앤다.

**결과**: 임계값은 corpus-of-one. 다음 fixture에서 재평가.

## 검증됨 — 한컴 PUA 글머리표 범위

한컴 폰트는 `①..⑳` (그리고 다른 열거 글리프) 를 PUA codepoint로 인코딩;
HCR Dotum / Batang 외에서는 tofu로 표시됨. BMP 범위 `U+F2B1+` 만 있을 것이라
예상했으나 TRL fixture의 `① 과제 개요` 는 실제로 **Supplementary PUA-A**
`U+F02B1` 로 도착. 두 범위가 동일 offset에서 같은 글리프를 인코딩:

- `U+F02B1..U+F02C4` → `U+2460..U+2473` (`①..⑳`)
- `U+F2B1..U+F2C4`   → 동일

Supplementary 범위가 최신 hwp.exe 출력; BMP 형태는 구 문서.

**미해결 질문**: `㉠..㉭`, 괄호형 `⑴..⒇` 등 추가 PUA 매핑은 fixture-driven
발견 방식 — 나타나면 추가.

## 미해결 — row_span > 1 → bullet 경로; lossy-but-readable 대안 미탐색

GFM은 rowspan을 지원하지 않아서 세로 병합 셀이 있으면 표 전체가 bullet로
가버림. TRL fixture에서 가장 명확한 피해자는 §1 12×6 개요 표: `[8,0] span
2×1` 단 하나 때문에 12×6 전체가 bullet.

후보 접근:
- 병합 텍스트를 모든 row에 복제 (lossy: 레이블 시각 중복, 그러나 표 표시 가능)
- 방향 마커 `↑` / `←` / `↖` 으로 extension 채우기 (lossy: 병합 정보
  시각화되지만 원본 md → hwp 역방향에서 재병합 어려움)

실제 시도됨 (커밋 `7c6f81a`, 이후 reset) — `2026-04-22-rowspan-marker-
reverted.md` 참고.

## 미해결 — 단일 fixture corpus

라운드트립·품질 검증이 HWP 파일 1개에 의존. 두 번째 fixture (자유 서식
장문, 이미지 집중, 수식 집중, 암호화 문서) 를 추가하면 다음 버그 분류를 가장
빠르게 찾을 수 있음.
