//! HWPX container extraction. Thin wrapper over the `zip` crate that
//! pulls named parts out of the archive into owned byte buffers.
//!
//! HWPX is an OCF container (ZIP + XML) where the interesting
//! payloads are:
//!
//!   * `mimetype`                        — signature ("application/hwp+zip")
//!   * `META-INF/container.xml`          — points to the OPF root (always
//!                                          `Contents/content.hpf` in
//!                                          practice)
//!   * `Contents/content.hpf`            — OPF manifest / spine
//!   * `Contents/header.xml`             — styles, fonts, doc properties
//!                                          (DocInfo analogue)
//!   * `Contents/section{N}.xml`         — section bodies
//!   * `BinData/image{N}.{png,jpg,…}`    — embedded picture blobs
//!
//! This module stops at "open archive, fetch named parts". Parsing the
//! XML payloads is the next layer up (`section_xml.rs`).

use std::io::{Cursor, Read};

use hwp_transpiler_core::ir::IrError;

pub struct HwpxArchive {
    archive: zip::ZipArchive<Cursor<Vec<u8>>>,
}

impl HwpxArchive {
    /// Open the archive from an in-memory byte buffer. Uses an owned
    /// `Vec<u8>` rather than a borrowed slice because `ZipArchive`
    /// keeps the reader alive for the entry reads below.
    pub fn new(bytes: Vec<u8>) -> Result<Self, IrError> {
        let cursor = Cursor::new(bytes);
        let archive = zip::ZipArchive::new(cursor)
            .map_err(|e| IrError::Invalid(format!("open hwpx zip: {e}")))?;
        Ok(Self { archive })
    }

    /// Read a named entry's full contents into a `Vec<u8>`. Returns
    /// `IrError::Invalid` if the entry is missing or its stream can't
    /// be fully consumed. Callers for mandatory parts (e.g.
    /// `Contents/section0.xml`) should surface the error; optional
    /// parts should call `try_read_part`.
    pub fn read_part(&mut self, name: &str) -> Result<Vec<u8>, IrError> {
        let mut entry = self
            .archive
            .by_name(name)
            .map_err(|e| IrError::Invalid(format!("hwpx part {name:?}: {e}")))?;
        let mut out = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut out)
            .map_err(|e| IrError::Invalid(format!("read hwpx part {name:?}: {e}")))?;
        Ok(out)
    }

    /// Same as [`Self::read_part`] but returns `Ok(None)` when the
    /// entry is missing. Other I/O errors still propagate.
    pub fn try_read_part(&mut self, name: &str) -> Result<Option<Vec<u8>>, IrError> {
        match self.archive.by_name(name) {
            Ok(mut entry) => {
                let mut out = Vec::with_capacity(entry.size() as usize);
                entry.read_to_end(&mut out).map_err(|e| {
                    IrError::Invalid(format!("read hwpx part {name:?}: {e}"))
                })?;
                Ok(Some(out))
            }
            Err(zip::result::ZipError::FileNotFound) => Ok(None),
            Err(e) => Err(IrError::Invalid(format!(
                "hwpx part {name:?}: {e}"
            ))),
        }
    }

    /// Enumerate every entry name in the archive's central directory.
    /// Used to discover `Contents/section{N}.xml` without hard-coding
    /// a count, and to pick up `BinData/*` entries at read time.
    pub fn entry_names(&self) -> Vec<String> {
        self.archive.file_names().map(|s| s.to_string()).collect()
    }
}
