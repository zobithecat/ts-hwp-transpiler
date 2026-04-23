//! HWP5 writer. Emits typed streams from the IR, then flushes remaining
//! `unknown_streams` verbatim.
//!
//! Each stream owns a verbatim cache field (`stream_bytes`) that wins over
//! re-encoding when present — guarantees byte-equal round-trip for
//! unmutated documents. Clearing the cache (e.g. via `DocInfo::records_mut`)
//! forces re-encoding.

use hwp_transpiler_core::ir::{DocInfo, IrDocument, IrError, Section, Writer};
use std::io::Write as IoWrite;

use super::{ole, streams};

#[derive(Default)]
pub struct HwpWriter;

impl Writer for HwpWriter {
    fn write(&mut self, doc: &IrDocument) -> Result<Vec<u8>, IrError> {
        let mut comp = ole::new_in_memory()?;

        // Typed: /FileHeader
        write_stream(
            &mut comp,
            streams::file_header::STREAM_NAME,
            &streams::file_header::emit(&doc.header),
        )?;

        // Typed: /DocInfo
        let doc_info_bytes = encode_doc_info(&doc.doc_info, doc.header.compressed())?;
        write_stream(&mut comp, streams::doc_info::STREAM_NAME, &doc_info_bytes)?;

        // Typed: /BodyText/Section{N}
        comp.create_storage_all("/BodyText").map_err(|e| {
            IrError::Invalid(format!("create storage /BodyText: {e}"))
        })?;
        for (i, section) in doc.sections.iter().enumerate() {
            let path = format!("/BodyText/Section{i}");
            let bytes = encode_section(section, doc.header.compressed())?;
            write_stream(&mut comp, &path, &bytes)?;
        }

        // Typed: /BinData/<id> — embedded image / OLE blobs. Reader peels
        // these off `unknown_streams`; writer puts them back. The
        // `bin_data` field on `IrDocument` is the source of truth.
        if !doc.bin_data.is_empty() {
            comp.create_storage_all("/BinData").map_err(|e| {
                IrError::Invalid(format!("create storage /BinData: {e}"))
            })?;
            for entry in &doc.bin_data {
                let path = format!("/BinData/{}", entry.id);
                write_stream(&mut comp, &path, &entry.bytes)?;
            }
        }

        // Passthrough: anything not yet typed
        for (path, bytes) in &doc.unknown_streams {
            if is_typed_path(path) {
                continue;
            }
            if let Some(parent) = parent_storage_of(path) {
                comp.create_storage_all(&parent).map_err(|e| {
                    IrError::Invalid(format!("create storage {parent}: {e}"))
                })?;
            }
            write_stream(&mut comp, path, bytes)?;
        }

        Ok(ole::finalize(comp)?)
    }
}

fn encode_doc_info(info: &DocInfo, compressed: bool) -> Result<Vec<u8>, IrError> {
    if let Some(bytes) = &info.stream_bytes {
        return Ok(bytes.clone());
    }
    let raw = streams::record::emit_all(&info.raw_records);
    if compressed {
        streams::doc_info::compress(&raw)
    } else {
        Ok(raw)
    }
}

fn encode_section(section: &Section, compressed: bool) -> Result<Vec<u8>, IrError> {
    if let Some(bytes) = &section.stream_bytes {
        return Ok(bytes.clone());
    }
    // Verbatim cache cleared — caller mutated the section. Re-encode
    // from paragraphs + section.raw_records, syncing each paragraph's
    // typed `header` back into its first PARA_HEADER record along the
    // way (see `streams::body_text::emit_section`).
    streams::body_text::emit_section(section, compressed)
}

fn write_stream(
    comp: &mut cfb::CompoundFile<std::io::Cursor<Vec<u8>>>,
    path: &str,
    bytes: &[u8],
) -> Result<(), IrError> {
    let mut s = comp
        .create_stream(path)
        .map_err(|e| IrError::Invalid(format!("create stream {path}: {e}")))?;
    s.write_all(bytes)?;
    Ok(())
}

fn is_typed_path(path: &str) -> bool {
    path == streams::file_header::STREAM_NAME
        || path == streams::doc_info::STREAM_NAME
        || streams::body_text::section_index_of(path).is_some()
        || path.starts_with("/BinData/")
}

fn parent_storage_of(path: &str) -> Option<String> {
    let trimmed = path.trim_start_matches('/');
    trimmed.rfind('/').map(|i| format!("/{}", &trimmed[..i]))
}
