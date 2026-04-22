# HWP5 스펙 노트

실제 HWP 파일 또는 hwplib 소스로 검증한 binary-format 사실의 라이브
통합 모음. HWP5 공식 스펙은 여러 영역이 불완전 — 여기 적힌 건 우리가
*아는* 것이지 hwp.exe 가 다르게 동작할 수 있는 가능성을 포함하진 않음.

새 사실이 발견되면 업데이트. 그 사실을 surface한 저널 엔트리도 인용할
것. *왜 그걸 알게 됐나*는 저널의 영역.

---

## 컨테이너와 framing

### `/FileHeader` (256 B 고정, 비압축)

```
0..32   signature  ("HWP Document File\0…")
32..36  version     u32 LE
36..40  flags       u32 LE  (bit 0 = compressed,
                             bit 1 = encrypted,
                             bit 2 = distribute)
40..256 reserved   216 B (라이선스 / encrypt / distribute info — 불투명)
```

### Record TLV (DocInfo + BodyText 섹션 공유)

헤더는 4 bytes, bit-packed little-endian:

```
bits  0..10   tag      (10 bits)
bits 10..20   level    (10 bits) — 중첩 깊이
bits 20..32   size     (12 bits) — payload 길이 (bytes)
                                  size == 0xFFF 이면 다음 4바이트가 u32 LE 실제 size
```

### 압축 (DocInfo + 각 BodyText section)

`FileHeader.flags & 0x01` 가 set이면 스트림 payload는 **raw DEFLATE
(RFC 1951, `nowrap = true`)**. *zlib 헤더 없음, checksum 없음.* hwplib
는 Java `Deflater(level, nowrap=true)` 사용; 우리는 flate2 의
`DeflateDecoder` / `DeflateEncoder` 사용.

**주의**: DEFLATE 출력은 implementation-defined — flate2 와
java.util.zip 이 같은 입력에 대해 byte-identical 압축 바이트를 생성하지
않음. 라운드트립 byte-equality는 `stream_bytes` verbatim 캐시 경로로
보장 (재인코딩 아님). 타입드 레코드를 mutate 하면 캐시 clear되어
재인코딩 트리거 — 이 시점부터 원본과 바이트 분기.

---

## DocInfo record 태그

| tag    | 이름                  | IR 타입화? |
|--------|-----------------------|------------|
| 0x0010 | DOCUMENT_PROPERTIES   | yes        |
| 0x0011 | ID_MAPPINGS           | yes (내부 사용) |
| 0x0012 | BIN_DATA              | yes        |
| 0x0013 | FACE_NAME             | yes (×7 슬롯) |
| 0x0014 | BORDER_FILL           | yes        |
| 0x0015 | CHAR_SHAPE            | yes        |
| 0x0016 | TAB_DEF               | no         |
| 0x0017 | NUMBERING             | no         |
| 0x0018 | BULLET                | no         |
| 0x0019 | PARA_SHAPE            | yes        |
| 0x001A | STYLE                 | yes        |
| 0x001B | DOC_DATA              | no         |
| 0x001C | DISTRIBUTE_DOC_DATA   | no         |
| 0x001E | COMPATIBLE_DOCUMENT   | no         |
| 0x001F | LAYOUT_COMPATIBILITY  | no         |
| 0x0020 | TRACK_CHANGE_INFO     | no — passthrough이 의도 |
| 0x005C | MEMO_SHAPE            | no         |
| 0x005E | FORBIDDEN_CHAR        | no         |
| 0x0060 | TRACK_CHANGE          | no         |
| 0x0061 | TRACK_CHANGE_AUTHOR   | no         |

## BodyText record 태그

| tag    | 이름                       | IR 타입화? |
|--------|----------------------------|------------|
| 0x0042 | PARA_HEADER                | yes        |
| 0x0043 | PARA_TEXT                  | yes        |
| 0x0044 | PARA_CHAR_SHAPE            | yes        |
| 0x0045 | PARA_LINE_SEG              | yes        |
| 0x0046 | PARA_RANGE_TAG             | no         |
| 0x0047 | CTRL_HEADER                | partial — code 캡처, table 외 body는 Unknown |
| 0x0048 | LIST_HEADER (cell)         | yes        |
| 0x0049 | PAGE_DEF                   | no         |
| 0x004A | FOOTNOTE_SHAPE             | no         |
| 0x004B | PAGE_BORDER_FILL           | no         |
| 0x004C | SHAPE_COMPONENT            | partial — gsoId만 (picture 감지용) |
| 0x004D | TABLE                      | yes        |
| 0x0055 | SHAPE_COMPONENT_PICTURE    | partial — binItemID만 |

---

## 특정 record 레이아웃

### LIST_HEADER (cell variant, 38 B 고정 + optional trailer)

출처: hwplib `reader/.../tbl/ForCell.java::listHeader`.

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

**함정**: `paraCount` 가 sInt4 (u16 아님); trailer의 optional ×9 bytes
때문에 offset-from-end 파싱은 unsafe — offset-from-start로 읽을 것.
버그 history는 `2026-04-22-cell-parser-and-markdown-export.md` 참고.

### GSO 그림 체인 (BodyText)

본문의 그림은 `CTRL_HEADER` 를 root로 한 3-record 체인:

```
level n     CTRL_HEADER       code "gso ", body offset 12,16에 width/height
  level n+1 [LIST_HEADER]     optional caption 컨테이너
  level n+1 [CTRL_DATA]       optional ctrl data
  level n+1 SHAPE_COMPONENT   첫 4 바이트 (LE-reversed) = gsoId
                              "$pic" → picture; "$lin"/"$rec"/etc. → 다른 도형
    level n+2 [CTRL_DATA]     optional
    level n+2 SHAPE_COMPONENT_PICTURE  PictureInfo @ offset 68
                                       binItemID (u16 LE) @ offset 71
```

`SHAPE_COMPONENT_PICTURE`는 형제 `SHAPE_COMPONENT` 보다 한 레벨 **깊다**
(도형의 자식이지 peer가 아님). `SHAPE_COMPONENT` 시작의 GSO ID
discriminator는 little-endian이라 디스크에서 "$pic"는 `c`,`i`,`p`,`$`로
나타남 — `CTRL_HEADER` code와 동일 컨벤션 (`ctrl_header::display_code`
가 처리).

`binItemID` 는 `DocInfo.bin_data[i].bin_data_id` 와 cross-reference하여
실제 `/BinData/BIN<id>.<ext>` 스트림으로 resolve.

### BinData (DocInfo tag 0x0012)

출처: hwplib `reader/docinfo/ForBinData.java`.

```
property    uInt2          # low nibble = type (Link 0 / Embedding 1 / Storage 2)
                            # 그 다음 nibble = compression hint
                            # 그 다음 = status; top = reserved
if type == Link:
    absolute_path    UTF-16LE (u16 code-unit prefix)
    relative_path    UTF-16LE
if type == Embedding | Storage:
    bin_data_id      uInt2   # /BinData/BIN<id> 스트림 basename 매칭
    extension        UTF-16LE
```

`/BinData/BIN0001.png`, `/BinData/BIN0002.jpg`, 등 — bin_data_id는 숫자
suffix; extension은 스트림 이름에 붙음.

### UTF-16LE 문자열

표준 HWP 컨벤션: u16 code-unit count prefix, 그 다음 그만큼의 u16 LE
code units. surrogate pair는 두 u16으로 유지. 빈 문자열 = `0x0000
0x0000` 길이 prefix만.

---

## PUA 문자 변환

한컴은 일부 글리프를 PUA codepoint로 인코딩; HCR Dotum / Batang 외에는
tofu로 표시됨. `clean_text` 에서 정규화.

| 한컴 범위                     | 표준 타깃         | 글리프 |
|-------------------------------|-------------------|--------|
| U+F2B1..U+F2C4 (BMP)          | U+2460..U+2473    | ①..⑳   |
| U+F02B1..U+F02C4 (Supp PUA-A) | U+2460..U+2473    | ①..⑳ (최신 hwp.exe) |

다른 매핑도 분명 필요 (`㉠..㉭`, 괄호형 `⑴..⒇`, 등) — fixture에서 tofu로
나타나면 추가.

---

## OLE 컨테이너 (CFB)

`cfb` Rust crate (mdsteele/rust-cfb) 사용. 예상되는 스트림:

- `/FileHeader` — 필수, 항상 비압축
- `/DocInfo` — 필수, 압축 여부는 FileHeader.flags 따름
- `/BodyText/Section{N}` — 필수, 동일 압축 규칙
- `/BinData/BIN<id>.<ext>` — optional, 스트림별 압축 규칙 미결정
  (현재 passthrough; 우리가 직접 decompress하지 않음)
- `/PrvText`, `/PrvImage`, `/DocOptions/...`, `/Scripts/...`,
  `/DistributeDocData`, `/ViewText/...` — `unknown_streams` 통한
  passthrough

스트림 순서는 HWP 읽기에서 무관하지만, `/BodyText/Section{N}` 은
`IrDocument.sections` 가 source 순서와 매칭하려면 numerical 정렬 필요.
