//! `/FileHeader` — fixed 256-byte uncompressed header.
//!
//! Layout (HWP5 spec):
//!   0..32   signature (b"HWP Document File" + padding)
//!   32..36  version         u32 LE, packed (major, minor, revision, patch)
//!   36..40  flags           u32 LE — bit 0 compressed, bit 1 encrypted,
//!                                    bit 2 distribute-save, ...
//!   40..256 reserved / sub-fields (license/encrypt/distribute info)

use hwp_transpiler_core::ir::{FileHeader, IrError};

pub const STREAM_NAME: &str = "/FileHeader";
pub const SIZE: usize = 256;

pub fn parse(bytes: &[u8]) -> Result<FileHeader, IrError> {
    if bytes.len() != SIZE {
        return Err(IrError::Invalid(format!(
            "FileHeader expected {SIZE} bytes, got {}",
            bytes.len()
        )));
    }
    let mut signature = [0u8; 32];
    signature.copy_from_slice(&bytes[0..32]);
    let version = u32::from_le_bytes(bytes[32..36].try_into().unwrap());
    let flags = u32::from_le_bytes(bytes[36..40].try_into().unwrap());
    let reserved = bytes[40..SIZE].to_vec();

    Ok(FileHeader { signature, version, flags, reserved })
}

pub fn emit(hdr: &FileHeader) -> [u8; SIZE] {
    let mut out = [0u8; SIZE];
    out[0..32].copy_from_slice(&hdr.signature);
    out[32..36].copy_from_slice(&hdr.version.to_le_bytes());
    out[36..40].copy_from_slice(&hdr.flags.to_le_bytes());
    let n = hdr.reserved.len().min(SIZE - 40);
    out[40..40 + n].copy_from_slice(&hdr.reserved[..n]);
    out
}
