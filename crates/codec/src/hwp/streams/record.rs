//! HWP5 record TLV codec. Every DocInfo / BodyText record carries a 4-byte
//! little-endian header packed as:
//!
//!   bits  0..10   tag       (0..=1023)
//!   bits 10..20   level     (nesting depth)
//!   bits 20..32   size      (if == 0xFFF, next u32 LE is the real size)
//!
//! Payload follows immediately. Records concatenate — no framing bytes
//! between them.

use hwp_transpiler_core::ir::{IrError, Record};

const INLINE_SIZE_MAX: u32 = 0xFFF;

pub fn parse_all(input: &[u8]) -> Result<Vec<Record>, IrError> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < input.len() {
        out.push(parse_one(input, &mut pos)?);
    }
    Ok(out)
}

pub fn parse_one(input: &[u8], pos: &mut usize) -> Result<Record, IrError> {
    if *pos + 4 > input.len() {
        return Err(IrError::Invalid(format!(
            "record header truncated at offset {}",
            *pos
        )));
    }
    let header = u32::from_le_bytes(input[*pos..*pos + 4].try_into().unwrap());
    *pos += 4;

    let tag = (header & 0x3FF) as u16;
    let level = ((header >> 10) & 0x3FF) as u16;
    let mut size = (header >> 20) & 0xFFF;
    if size == INLINE_SIZE_MAX {
        if *pos + 4 > input.len() {
            return Err(IrError::Invalid(format!(
                "extended size truncated at offset {}",
                *pos
            )));
        }
        size = u32::from_le_bytes(input[*pos..*pos + 4].try_into().unwrap());
        *pos += 4;
    }
    let end = *pos + size as usize;
    if end > input.len() {
        return Err(IrError::Invalid(format!(
            "record payload truncated: tag={:#x} size={} remaining={}",
            tag,
            size,
            input.len() - *pos
        )));
    }
    let data = input[*pos..end].to_vec();
    *pos = end;
    Ok(Record { tag, level, data })
}

pub fn emit_all(records: &[Record]) -> Vec<u8> {
    let mut out = Vec::new();
    for r in records {
        emit_one(&mut out, r);
    }
    out
}

pub fn emit_one(out: &mut Vec<u8>, r: &Record) {
    let size = r.data.len() as u32;
    let (inline_size, ext) = if size >= INLINE_SIZE_MAX {
        (INLINE_SIZE_MAX, Some(size))
    } else {
        (size, None)
    };
    let header = (r.tag as u32 & 0x3FF)
        | ((r.level as u32 & 0x3FF) << 10)
        | ((inline_size & 0xFFF) << 20);
    out.extend_from_slice(&header.to_le_bytes());
    if let Some(s) = ext {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out.extend_from_slice(&r.data);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_record_roundtrips() {
        let input = Record { tag: 0x10, level: 0, data: vec![1, 2, 3, 4] };
        let bytes = emit_all(&[input.clone()]);
        assert_eq!(bytes.len(), 4 + 4);
        let parsed = parse_all(&bytes).unwrap();
        assert_eq!(parsed, vec![input]);
    }

    #[test]
    fn large_record_uses_extended_size() {
        let data = vec![0xAB; 0x1000];
        let input = Record { tag: 0x15, level: 2, data };
        let bytes = emit_all(&[input.clone()]);
        assert_eq!(bytes.len(), 4 + 4 + 0x1000);
        let parsed = parse_all(&bytes).unwrap();
        assert_eq!(parsed, vec![input]);
    }

    #[test]
    fn boundary_size_exactly_4094_stays_inline() {
        let input = Record { tag: 0x11, level: 0, data: vec![0xCC; 0xFFE] };
        let bytes = emit_all(&[input.clone()]);
        assert_eq!(bytes.len(), 4 + 0xFFE);
        let parsed = parse_all(&bytes).unwrap();
        assert_eq!(parsed, vec![input]);
    }

    #[test]
    fn stream_of_records_concatenates() {
        let rs = vec![
            Record { tag: 0x10, level: 0, data: vec![1] },
            Record { tag: 0x11, level: 1, data: vec![2, 3] },
            Record { tag: 0x12, level: 0, data: vec![] },
        ];
        let bytes = emit_all(&rs);
        let parsed = parse_all(&bytes).unwrap();
        assert_eq!(parsed, rs);
    }

    #[test]
    fn truncated_header_errors() {
        assert!(parse_all(&[0x00]).is_err());
    }

    #[test]
    fn truncated_payload_errors() {
        let header = 0x10u32 | (0u32 << 10) | (10u32 << 20);
        let mut bytes = header.to_le_bytes().to_vec();
        bytes.extend_from_slice(&[0xAA, 0xBB]);
        assert!(parse_all(&bytes).is_err());
    }
}
