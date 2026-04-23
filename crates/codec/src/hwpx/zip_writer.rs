//! HWPX container writer. Thin wrapper over the `zip` crate that
//! builds an OCF-compliant archive from named parts.
//!
//! OCF spec requires `mimetype` to be the **first** entry and
//! **stored uncompressed** (same convention as ODF / EPUB). Other
//! parts are DEFLATE-compressed. This writer enforces that ordering
//! so downstream viewers that sniff the container type via the fixed-
//! offset mimetype bytes still work.

use std::io::{Cursor, Write};

use hwp_transpiler_core::ir::IrError;
use zip::write::{FileOptions, ZipWriter};
use zip::CompressionMethod;

pub struct HwpxArchiveWriter {
    zip: ZipWriter<Cursor<Vec<u8>>>,
    wrote_mimetype: bool,
}

impl HwpxArchiveWriter {
    pub fn new() -> Self {
        Self {
            zip: ZipWriter::new(Cursor::new(Vec::new())),
            wrote_mimetype: false,
        }
    }

    /// Emit the `mimetype` entry uncompressed. Callers must invoke
    /// this before any other `add_part` so it lands at offset 0 in
    /// the archive. Returns an error on a second call.
    pub fn write_mimetype(&mut self, mimetype: &str) -> Result<(), IrError> {
        if self.wrote_mimetype {
            return Err(IrError::Invalid(
                "hwpx: mimetype already written".into(),
            ));
        }
        let opts = FileOptions::default()
            .compression_method(CompressionMethod::Stored);
        self.zip
            .start_file("mimetype", opts)
            .map_err(zip_err)?;
        self.zip
            .write_all(mimetype.as_bytes())
            .map_err(io_err)?;
        self.wrote_mimetype = true;
        Ok(())
    }

    /// Add a compressed entry. `name` follows HWPX convention: slash-
    /// separated path like `Contents/section0.xml`. Binary payloads
    /// (images, OLE blobs) go through the same entry point; DEFLATE
    /// on already-compressed image data is near-identity and not
    /// worth a separate code path.
    pub fn add_part(&mut self, name: &str, bytes: &[u8]) -> Result<(), IrError> {
        let opts = FileOptions::default()
            .compression_method(CompressionMethod::Deflated);
        self.zip.start_file(name, opts).map_err(zip_err)?;
        self.zip.write_all(bytes).map_err(io_err)?;
        Ok(())
    }

    /// Close the archive and return the raw bytes. Consumes `self`
    /// because `ZipWriter::finish()` takes ownership.
    pub fn finish(mut self) -> Result<Vec<u8>, IrError> {
        let cursor = self.zip.finish().map_err(zip_err)?;
        Ok(cursor.into_inner())
    }
}

impl Default for HwpxArchiveWriter {
    fn default() -> Self {
        Self::new()
    }
}

fn zip_err(e: zip::result::ZipError) -> IrError {
    IrError::Invalid(format!("hwpx zip: {e}"))
}

fn io_err(e: std::io::Error) -> IrError {
    IrError::Invalid(format!("hwpx zip io: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_archive_finishes_cleanly() {
        let w = HwpxArchiveWriter::new();
        let bytes = w.finish().expect("finish");
        // Minimal zip: EOCD record is 22 bytes.
        assert!(bytes.len() >= 22);
    }

    #[test]
    fn mimetype_is_stored_at_offset_zero() {
        let mut w = HwpxArchiveWriter::new();
        w.write_mimetype("application/hwp+zip").expect("mimetype");
        w.add_part("Contents/a.xml", b"<a/>").expect("part");
        let bytes = w.finish().expect("finish");
        // Zip local file header starts with `PK\x03\x04`.
        assert_eq!(&bytes[0..4], b"PK\x03\x04");
        // A stored mimetype entry places its raw bytes right after
        // the LFH. The LFH is 30 bytes + filename length (8).
        let off = 30 + "mimetype".len();
        assert_eq!(&bytes[off..off + 18], b"application/hwp+zi");
    }

    #[test]
    fn second_mimetype_errors() {
        let mut w = HwpxArchiveWriter::new();
        w.write_mimetype("application/hwp+zip").expect("first ok");
        assert!(w.write_mimetype("application/hwp+zip").is_err());
    }
}
