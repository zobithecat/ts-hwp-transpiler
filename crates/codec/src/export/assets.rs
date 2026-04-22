//! Sidecar asset dump.
//!
//! Writes every `BinaryEntry` in `IrDocument.bin_data` to a sibling
//! directory so the Markdown export's `![](…)` links actually resolve on
//! disk. The directory layout mirrors HWP5's `/BinData/` stream names
//! verbatim — reader strips the prefix and keeps the leaf (`BIN0001.png`),
//! so `<assets_dir>/BIN0001.png` is what both this dumper writes and
//! `markdown::emit_picture` refers to.

use hwp_transpiler_core::ir::IrDocument;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Write every `BinaryEntry` to `<dir>/<entry.id>`. Creates `dir` (and
/// parents) if missing. Existing files are overwritten — a deliberate
/// choice so re-running `hwp-to-md` produces a clean asset set rather
/// than appending stale files. Returns the list of paths written (useful
/// for CLI status reporting and tests).
///
/// Rejects entries whose id contains a path separator, is empty, or
/// starts with `.` — binary entry ids come from OLE stream leaf names
/// (`/BinData/BIN0001.png` → `BIN0001.png`) and should already be safe,
/// but `dir` is user-controllable so we defensively guard against any
/// traversal attempt that could leak from a malformed fixture.
pub fn dump_assets(doc: &IrDocument, dir: &Path) -> io::Result<Vec<PathBuf>> {
    fs::create_dir_all(dir)?;
    let mut written = Vec::with_capacity(doc.bin_data.len());
    for entry in &doc.bin_data {
        let name = sanitise_id(&entry.id)?;
        let path = dir.join(name);
        fs::write(&path, &entry.bytes)?;
        written.push(path);
    }
    Ok(written)
}

fn sanitise_id(id: &str) -> io::Result<&str> {
    if id.is_empty()
        || id.starts_with('.')
        || id.contains('/')
        || id.contains('\\')
        || id.contains('\0')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsafe binary entry id: {id:?}"),
        ));
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hwp_transpiler_core::ir::BinaryEntry;

    fn doc_with_entries(entries: Vec<BinaryEntry>) -> IrDocument {
        let mut doc = IrDocument::default();
        doc.bin_data = entries;
        doc
    }

    fn tmp_dir(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "hwp-transpiler-assets-test-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn writes_each_entry_to_id_named_file() {
        let dir = tmp_dir("basic");
        let doc = doc_with_entries(vec![
            BinaryEntry {
                id: "BIN0001.png".into(),
                mime: Some("image/png".into()),
                bytes: b"\x89PNG fake".to_vec(),
            },
            BinaryEntry {
                id: "BIN0002.jpg".into(),
                mime: Some("image/jpeg".into()),
                bytes: b"\xFF\xD8 fake".to_vec(),
            },
        ]);

        let written = dump_assets(&doc, &dir).expect("dump");
        assert_eq!(written.len(), 2);

        let png = fs::read(dir.join("BIN0001.png")).unwrap();
        assert_eq!(png, b"\x89PNG fake");
        let jpg = fs::read(dir.join("BIN0002.jpg")).unwrap();
        assert_eq!(jpg, b"\xFF\xD8 fake");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn creates_missing_parent_dirs() {
        let dir = tmp_dir("nested").join("a").join("b");
        let doc = doc_with_entries(vec![BinaryEntry {
            id: "BIN0001.png".into(),
            mime: None,
            bytes: b"x".to_vec(),
        }]);

        dump_assets(&doc, &dir).expect("dump");
        assert!(dir.join("BIN0001.png").is_file());

        fs::remove_dir_all(&dir.parent().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn overwrites_existing_files() {
        let dir = tmp_dir("overwrite");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("BIN0001.png"), b"STALE").unwrap();

        let doc = doc_with_entries(vec![BinaryEntry {
            id: "BIN0001.png".into(),
            mime: None,
            bytes: b"FRESH".to_vec(),
        }]);
        dump_assets(&doc, &dir).expect("dump");

        assert_eq!(fs::read(dir.join("BIN0001.png")).unwrap(), b"FRESH");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_bin_data_still_creates_dir() {
        let dir = tmp_dir("empty");
        let doc = doc_with_entries(vec![]);
        let written = dump_assets(&doc, &dir).expect("dump");
        assert!(written.is_empty());
        assert!(dir.is_dir());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_traversal_ids() {
        let dir = tmp_dir("traversal");
        for bad in ["", "../etc/passwd", "sub/file.png", "a\\b.png", ".hidden"] {
            let doc = doc_with_entries(vec![BinaryEntry {
                id: bad.into(),
                mime: None,
                bytes: b"x".to_vec(),
            }]);
            let err = dump_assets(&doc, &dir).expect_err(&format!(
                "expected error for bad id {bad:?}"
            ));
            assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        }
        fs::remove_dir_all(&dir).ok();
    }
}
