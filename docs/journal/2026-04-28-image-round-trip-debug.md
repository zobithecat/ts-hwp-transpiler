# 2026-04-28 — 이미지 round-trip viewer 호환성 디버그

## 맥락

오전에 ship한 MD 에셋 파이프라인(저널 `2026-04-28-md-asset-pipeline`)은
"그림 bytes 가 .md 거쳐도 살아남는다"까지만 검증함. 사용자가 실제 한컴
viewer에서 round-trip된 .hwpx를 열어보고 단계별로 무엇이 문제인지
보고해 주면서 viewer 호환성 결함 7개가 연쇄적으로 드러났음. 이 저널은
그 디버그 사이클의 영구 기록.

테스트 fixture: `[1.28.수+석간]+장애인+정보접근권+강화를+위한+'장벽+없는+
무인정보단말기'+의무화+전면+시행(1.28.).hwpx` (한국 보건복지부 보도자료,
JPG 그림 2개, 셀 테두리 다수).

## 발견 1 — `section_writer`가 `<hp:pic>` 자체를 emit 안 함

```
사용자: "그림은 안나와"
```

`section_writer.rs::emit_run_with_range`의 control 분기:
```rust
match &ctrl.kind {
    ControlKind::Table(t) => emit_table(t, out),
    _ => {} // documented gap — pictures drop silently
}
```

PictureControl은 IR에 들어왔으나 XML 레벨에서 사라짐. **fix**: 28-children
짜리 minimum-viable `<hp:pic>` emitter 추가 — `<hp:offset>` / `<hp:orgSz>`
/ `<hp:curSz>` / `<hp:flip>` / `<hp:rotationInfo>` / `<hp:renderingInfo>`
(transMatrix / scaMatrix / rotMatrix) / `<hc:img binaryItemIDRef>` /
`<hp:imgRect>` / `<hp:imgClip>` / `<hp:inMargin>` / `<hp:imgDim>` /
`<hp:effects/>` / `<hp:sz>` / `<hp:pos>` / `<hp:outMargin>`.

## 발견 2 — `parse_picture`가 `<hp:orgSz>` 무시 → width/height = 0

원본 .hwpx 들이 `<hp:curSz width="0" height="0"/>` + `<hp:orgSz width="W"
height="H"/>` 패턴을 흔하게 씀. 우리 파서는 `<hp:curSz>`만 읽어서
`PictureControl.width_hwpu = 0`, `height_hwpu = 0`. 결과 `<hp:pic>`이 1×1
HWPUNIT으로 emit돼 viewer가 점 크기 그림 그림.

**fix**: `<hp:orgSz>` fallback. curSz 가 0이면 orgSz 값 사용.

## 발견 3 — `<hp:imgClip>` `right=0 bottom=0` → "0×0 클립"

원본은 `imgClip left=0 right=W top=0 bottom=H`로 클립 영역 명시. 우리는 0/0/0/0
이라 일부 viewer가 "전체 클립=0×0"으로 해석해 그림 안 그림. 또
`<hp:imgDim>`도 누락, `<hp:effects/>` empty placeholder도 누락.

**fix**: imgClip을 orgSz 값으로, imgDim 추가, effects empty placeholder
추가.

## 발견 4 — `<hp:run>` 안에서 `<hp:t>` 가 `<hp:pic>` 보다 앞

```
사용자: "obj어쩌고는 안떠 근데 여전히 이미지는 안뜸"
```

원본:
```xml
<hp:run charPrIDRef="38">
  <hp:pic …/>
  <hp:t>광부</hp:t>
</hp:run>
```

우리:
```xml
<hp:run>
  <hp:t>￼</hp:t>
  <hp:pic …/>
</hp:run>
```

추가로 `\u{FFFC}` (object replacement char — IR이 picture 위치 placeholder로
사용)가 `<hp:t>` 안에 그대로 들어가 viewer가 literal "obj" 글리프로 렌더 +
picture를 unknown OLE로 분류.

**fix**: `emit_run_with_range`가
- 컨트롤(pic/tbl) 먼저 emit
- 그 다음 `\u{FFFC}` 필터링한 visible text만 `<hp:t>` 출력
- visible text 비어 있으면 `<hp:t>` 자체 생략

## 발견 5 — encode_for_md가 JPG 강제로 PNG 재인코딩

`asset_pipeline::encode_for_md` default `dpi = Some(72)` →
- JPG 4016B → 디코드 → PNG 13804B로 부풀음 (lossless 보존이지만 lossy 원본엔 무의미)
- BinaryEntry.id를 `image1.png`로 rename → BinData/image1.png
- 그러나 두 번째 fix 전엔 source_id 유지 → `BinData/image1.jpg`에 PNG 바이트 (mime 충돌)

**fix 1**: `renormalise_id_to_png` — bytes/extension 일치.

**fix 2** (사용자 "PNG가 HWPX에서 지원되는건 맞아???" 직후):
`EncodeOpts::default()` 를 `dpi = None` 으로 변경. **default = verbatim
원본 보존**. 디코드 안 함, mime 보존, id 보존, byte-equal round-trip.
LLM 컨텍스트 줄이고 싶으면 명시적 `--asset-dpi=72` (또는 36) 옵션.

## 발견 6 — content.hpf manifest 에 BinData 미등록

```
사용자: "여전히 안나와"
```

원본 content.hpf:
```xml
<opf:item id="image1" href="BinData/image1.jpg" media-type="image/jpg" isEmbeded="1"/>
```

우리 RT manifest엔 header / section0만. 한컴 viewer는 `binaryItemIDRef=
"image1"` 을 manifest의 `<opf:item id>`으로 매핑하므로 등록 안 되면
무음 스킵.

**fix**: `HwpxWriter`가 `Contents/content.hpf` 에 surgical splice. 
`doc.bin_data` 의 모든 entry에 대해
`<opf:item id="image1" href="BinData/image1.jpg" media-type="image/jpg" isEmbeded="1"/>`
삽입. 이미 있는 항목은 건드리지 않음.

## 발견 7 — `<hp:pic id="0" instid="0">` 모두 동일

다중 picture가 같은 outer id 공유 → 일부 viewer가 단일 객체로 처리해
한 장만 보이거나 둘 다 안 보임.

**fix**: deterministic unique id from bin_id —
`pic_id = 1_000_000 + bin_id`, `instid = 2_000_000 + bin_id`.

## 발견 8 — manifest mime / order 한컴 패턴과 다름

원본: `media-type="image/jpg"` (한컴 줄임), 순서 `header → image1 → image2
→ section0 → settings`. 우리: `image/jpeg` (표준), 순서 `header →
section0 → image1 → image2`.

**fix**: 
- `mime_for_manifest`가 `.jpg`/`.jpeg` → `image/jpg` (한컴식)
- splice anchor를 "첫 `id="section…"` 항목 직전"으로 변경 → 순서 정렬

## 결과

각 발견마다 별도 커밋으로 ship. 7개 fix 후:
- BinData/image1.jpg / image2.JPG: 원본과 byte-equal verbatim
- content.hpf manifest: header → image2 → image1 → section0 (한컴 형식)
- `<hp:pic>`: unique id / instid, 28 children, orgSz/imgClip/imgDim 정확
- `<hp:run>`: pic 먼저, U+FFFC 없음

사용자 viewer 결과는 push 시점에 미확인 — 검증 사이클 진행 중.

## 미해결 질문

- **남은 viewer 거부 원인** — 위 8개 정렬에도 사용자가 "안 나옴" 보고 시
  남은 차이는 무엇? 가능 후보:
  - `<hp:pic>` charPrIDRef 가 0인데 원본은 38 같은 specific id 사용
  - `<hp:p>` 의 paraPrIDRef / styleIDRef 영향
  - cell 안 paragraph id가 모두 0으로 통일된 문제
  - 한컴 viewer가 검사하는 별도 메타 (settings.xml, version.xml?)
- **Lossy 옵션 미지원** — `--asset-dpi=N` 으로 PNG 재인코딩하는 lossy
  branch 는 round-trip 안전성 깨뜨려서 의도적 미지원. 사용 시 BinData
  filename rename + media-type 맞춤 필요 시 .png/jpg 모드 분기 더 필요.
- **gradation/image fill mutation** — 의도적 미지원 유지 (IR 측 typed
  field 없음).
- **viewer-specific 호환성** — 한컴 (HWPViewer / HWP+) / rhwp / Microsoft
  뷰어가 각각 다른 strict 검사. 만족 못 하는 element 가 있으면 round-trip
  무용. 실 viewer 로그 / 에러 메시지가 다음 디버그 사이클의 단서.
