//! GSO ("graphic shape object") picture decoding.
//!
//! HWP draws every graphic — line, rectangle, picture, text-art, etc. —
//! as a CTRL_HEADER with code "gso " followed by a SHAPE_COMPONENT
//! (which carries a 4-byte FOURCC discriminator), and then a
//! shape-specific record. For pictures the chain is:
//!
//! ```text
//! level n  : CTRL_HEADER  (code "gso ", body has width/height + offsets)
//! level n+1: [optional LIST_HEADER (caption), CTRL_DATA]
//! level n+1: SHAPE_COMPONENT  (first 4 bytes = gsoId == "$pic")
//! level n+1: [optional CTRL_DATA]
//! level n+1: SHAPE_COMPONENT_PICTURE  (PictureInfo block carries binItemID)
//! ```
//!
//! This module only handles the data extraction; the state-machine that
//! threads the records together lives in `body_text::parse_paragraph`.
//!
//! References:
//!   - hwplib `reader/.../gso/part/ForCtrlHeaderGso.java`
//!   - hwplib `reader/.../gso/part/ForShapeComponent.java`
//!   - hwplib `reader/.../gso/ForControlPicture.java`
//!   - hwplib `reader/docinfo/borderfill/ForFillInfo.java::pictureInfo`

/// FOURCC for picture shapes inside `SHAPE_COMPONENT`, after byte-order
/// normalisation. HWP encodes these as little-endian u32, so on disk the
/// four bytes for "$pic" appear as `c`,`i`,`p`,`$`. `parse_shape_component_id`
/// reverses to readable order before returning, so this constant stays as
/// the human-readable form. Same convention as `ctrl_header::display_code`.
pub const GSO_ID_PICTURE: [u8; 4] = *b"$pic";

/// Read width / height (HWPUNIT, both u32 LE) from a `CTRL_HEADER` whose
/// code is `"gso "`. Returns `None` if the body is too short.
///
/// CtrlHeaderGso layout (from hwplib `ForCtrlHeaderGso`):
///
/// ```text
/// 0..4    property      u32
/// 4..8    y_offset      u32
/// 8..12   x_offset      u32
/// 12..16  width         u32   ← here
/// 16..20  height        u32   ← here
/// 20..24  z_order       i32
/// 24..32  margins (u16 × 4)
/// 32..36  instance_id   u32
/// 36..    optional (page-divide flag, explanation string, ...)
/// ```
pub fn parse_gso_size(ctrl_header_body: &[u8]) -> Option<(u32, u32)> {
    if ctrl_header_body.len() < 20 {
        return None;
    }
    let width = u32::from_le_bytes(ctrl_header_body[12..16].try_into().ok()?);
    let height = u32::from_le_bytes(ctrl_header_body[16..20].try_into().ok()?);
    Some((width, height))
}

/// Read the 4-byte gsoId at the start of a `SHAPE_COMPONENT` record and
/// return it in human-readable order ("$pic" rather than the on-disk
/// little-endian "cip$"). Returns `None` if too short.
pub fn parse_shape_component_id(body: &[u8]) -> Option<[u8; 4]> {
    if body.len() < 4 {
        return None;
    }
    Some([body[3], body[2], body[1], body[0]])
}

/// Read `binItemID` (u16 LE) from a `SHAPE_COMPONENT_PICTURE` record's
/// PictureInfo block. The PictureInfo block lives at offset 68 (after
/// borders + four corner points + after-cutting margins + inner margins),
/// and within it `binItemID` follows three single-byte fields:
///
/// ```text
/// offset 68: brightness  i8
/// offset 69: contrast    i8
/// offset 70: effect      u8
/// offset 71: binItemID   u16 LE   ← here
/// ```
///
/// Returns `None` if the record body doesn't reach offset 73.
pub fn parse_picture_bin_id(body: &[u8]) -> Option<u16> {
    const OFFSET: usize = 71;
    if body.len() < OFFSET + 2 {
        return None;
    }
    Some(u16::from_le_bytes([body[OFFSET], body[OFFSET + 1]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_gso_size_extracts_offsets_12_and_16() {
        let mut body = vec![0u8; 36];
        body[12..16].copy_from_slice(&12_345u32.to_le_bytes());
        body[16..20].copy_from_slice(&67_890u32.to_le_bytes());
        assert_eq!(parse_gso_size(&body), Some((12_345, 67_890)));
    }

    #[test]
    fn parse_gso_size_short_body_returns_none() {
        assert_eq!(parse_gso_size(&[0u8; 18]), None);
    }

    #[test]
    fn shape_component_id_pic_is_dollar_p_i_c() {
        // On-disk bytes are LE-reversed, so "$pic" lives as "cip$".
        let body = b"cip$xxxxxxxx";
        assert_eq!(parse_shape_component_id(body), Some(*b"$pic"));
        assert_eq!(parse_shape_component_id(body), Some(GSO_ID_PICTURE));
    }

    #[test]
    fn picture_bin_id_extracted_from_offset_71() {
        // SHAPE_COMPONENT_PICTURE: pad up to offset 71 with arbitrary
        // bytes, then place a u16 LE binItemID = 7 there.
        let mut body = vec![0xCDu8; 80];
        body[71..73].copy_from_slice(&7u16.to_le_bytes());
        assert_eq!(parse_picture_bin_id(&body), Some(7));
    }

    #[test]
    fn picture_bin_id_short_body_returns_none() {
        assert_eq!(parse_picture_bin_id(&[0u8; 70]), None);
    }
}
