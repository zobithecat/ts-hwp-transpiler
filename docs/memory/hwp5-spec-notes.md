# HWP5 spec notes

Live consolidation of binary-format facts we've verified against
real-world HWP files or hwplib source. Public HWP5 spec is incomplete
in many areas, so what's here is what we know, not what hwp.exe might
do differently.

Update when a new fact lands. Cite the journal entry that surfaced it.
Don't put *why we know* here — that's journal territory.

---

## Containers & framing

### `/FileHeader` (256 B fixed, uncompressed)

```
0..32   signature  ("HWP Document File\0…")
32..36  version     u32 LE
36..40  flags       u32 LE  (bit 0 = compressed,
                             bit 1 = encrypted,
                             bit 2 = distribute)
40..256 reserved   216 B (license / encrypt / distribute info — opaque)
```

### Record TLV (DocInfo + BodyText sections share this)

Header is 4 bytes, bit-packed little-endian:

```
bits  0..10   tag      (10 bits)
bits 10..20   level    (10 bits) — nesting depth
bits 20..32   size     (12 bits) — payload length in bytes
                                  if size == 0xFFF → next 4 bytes are u32 LE size
```

### Compression (DocInfo + each BodyText section)

When `FileHeader.flags & 0x01` is set, the stream payload is **raw
DEFLATE (RFC 1951, `nowrap = true`)**. *No zlib header, no checksum.*
hwplib uses Java's `Deflater(level, nowrap=true)`; we use flate2's
`DeflateDecoder` / `DeflateEncoder`.

**Caveat**: DEFLATE output is implementation-defined — flate2 and
java.util.zip don't produce byte-identical compressed bytes for the
same input. Round-trip byte-equality goes through the `stream_bytes`
verbatim cache, not re-encoding. Mutating typed records clears the
cache and triggers re-encode, at which point bytes diverge from the
original.

---

## DocInfo record tags

| tag    | name                  | typed in IR? |
|--------|-----------------------|--------------|
| 0x0010 | DOCUMENT_PROPERTIES   | yes          |
| 0x0011 | ID_MAPPINGS           | yes (used internally) |
| 0x0012 | BIN_DATA              | yes          |
| 0x0013 | FACE_NAME             | yes (×7 slots) |
| 0x0014 | BORDER_FILL           | yes          |
| 0x0015 | CHAR_SHAPE            | yes          |
| 0x0016 | TAB_DEF               | no           |
| 0x0017 | NUMBERING             | no           |
| 0x0018 | BULLET                | no           |
| 0x0019 | PARA_SHAPE            | yes          |
| 0x001A | STYLE                 | yes          |
| 0x001B | DOC_DATA              | no           |
| 0x001C | DISTRIBUTE_DOC_DATA   | no           |
| 0x001E | COMPATIBLE_DOCUMENT   | no           |
| 0x001F | LAYOUT_COMPATIBILITY  | no           |
| 0x0020 | TRACK_CHANGE_INFO     | no — passthrough is the spec |
| 0x005C | MEMO_SHAPE            | no           |
| 0x005E | FORBIDDEN_CHAR        | no           |
| 0x0060 | TRACK_CHANGE          | no           |
| 0x0061 | TRACK_CHANGE_AUTHOR   | no           |

## BodyText record tags

| tag    | name                       | typed in IR? |
|--------|----------------------------|--------------|
| 0x0042 | PARA_HEADER                | yes          |
| 0x0043 | PARA_TEXT                  | yes          |
| 0x0044 | PARA_CHAR_SHAPE            | yes          |
| 0x0045 | PARA_LINE_SEG              | yes          |
| 0x0046 | PARA_RANGE_TAG             | no           |
| 0x0047 | CTRL_HEADER                | partial — code captured, body stays Unknown except for table |
| 0x0048 | LIST_HEADER (cell)         | yes          |
| 0x0049 | PAGE_DEF                   | no           |
| 0x004A | FOOTNOTE_SHAPE             | no           |
| 0x004B | PAGE_BORDER_FILL           | no           |
| 0x004C | SHAPE_COMPONENT            | **next** (Phase 2a-i) |
| 0x004D | TABLE                      | yes          |
| 0x0055 | SHAPE_COMPONENT_PICTURE    | **next** (Phase 2a-i) |

---

## Specific record layouts

### LIST_HEADER (cell variant, 38 B fixed + optional trailer)

Source: hwplib `reader/.../tbl/ForCell.java::listHeader`.

```
sInt4  paraCount        (4)
uInt4  property         (4)
uInt2  colIndex / rowIndex / colSpan / rowSpan          (4 × 2 = 8)
uInt4  width / height                                   (2 × 4 = 8)
uInt2  leftMargin / rightMargin / topMargin / bottomMargin  (8)
uInt2  borderFillId     (2)
uInt4  textWidth        (4)
                                                  (38 bytes fixed)
[opt]  uInt1 fieldNameFlag (0xff → ParameterSet) + 8-byte zero pad
```

**Gotcha**: `paraCount` is sInt4 not u16; the trailer's optional ×9
bytes mean offset-from-end parsing is unsafe — read offset-from-start.
See `2026-04-22-cell-parser-and-markdown-export.md` for the bug history.

### BinData (DocInfo tag 0x0012)

Source: hwplib `reader/docinfo/ForBinData.java`.

```
property    uInt2          # low nibble = type (Link 0 / Embedding 1 / Storage 2)
                            # next nibble = compression hint
                            # next = status; top = reserved
if type == Link:
    absolute_path    UTF-16LE (u16 code-unit prefix)
    relative_path    UTF-16LE
if type == Embedding | Storage:
    bin_data_id      uInt2   # matches /BinData/BIN<id> stream basename
    extension        UTF-16LE
```

`/BinData/BIN0001.png`, `/BinData/BIN0002.jpg`, etc. — the bin_data_id
is the numeric suffix; extension is appended to the stream name.

### UTF-16LE strings

Standard HWP convention: u16 code-unit count prefix, then that many
u16 LE code units. Surrogate pairs stay as two u16s. Empty string =
just a `0x0000 0x0000` length prefix.

---

## PUA character translations

Hancom uses PUA codepoints for some glyphs; outside HCR Dotum / Batang
they render as tofu. Normalise in `clean_text`.

| Hancom range            | Standard target   | Glyphs |
|-------------------------|-------------------|--------|
| U+F2B1..U+F2C4 (BMP)    | U+2460..U+2473    | ①..⑳   |
| U+F02B1..U+F02C4 (Supp PUA-A) | U+2460..U+2473 | ①..⑳ (newer hwp.exe) |

More mappings are surely needed (`㉠..㉭`, parenthesised `⑴..⒇`, etc.)
— add when a fixture surfaces them as tofu.

---

## OLE container (CFB)

We use the `cfb` Rust crate (mdsteele/rust-cfb). Streams expected:

- `/FileHeader` — required, always uncompressed
- `/DocInfo` — required, compression follows FileHeader.flags
- `/BodyText/Section{N}` — required, same compression rule
- `/BinData/BIN<id>.<ext>` — optional, compression rule undecided per
  stream (currently passthrough; we don't decompress these ourselves)
- `/PrvText`, `/PrvImage`, `/DocOptions/...`, `/Scripts/...`,
  `/DistributeDocData`, `/ViewText/...` — passthrough via
  `unknown_streams`

Stream order isn't significant for HWP read, but `/BodyText/Section{N}`
must be ordered numerically for our `IrDocument.sections` to match the
source order.
