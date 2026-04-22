//! HWP5 reader. Two-pass: FileHeader first (its `compressed` flag gates
//! DocInfo/BodyText decompression), then the rest. DocInfo and each
//! `/BodyText/Section{N}` stream flow into typed IR slots; remaining
//! streams stay in `IrDocument::unknown_streams` for verbatim round-trip.

use hwp_transpiler_core::ir::{DocInfo, FontFace, IrDocument, IrError, Reader};
use std::io::{Cursor, Read};

use super::streams;

#[derive(Default)]
pub struct HwpReader;

impl Reader for HwpReader {
    fn read(&mut self, input: &[u8]) -> Result<IrDocument, IrError> {
        let owned = input.to_vec();
        let mut comp = cfb::CompoundFile::open(Cursor::new(owned))
            .map_err(|e| IrError::Invalid(format!("open OLE compound: {e}")))?;

        let mut doc = IrDocument::default();

        // Pass 1: /FileHeader — `compressed` flag gates DocInfo/BodyText.
        let header_bytes = read_stream(&mut comp, streams::file_header::STREAM_NAME)?;
        doc.header = streams::file_header::parse(&header_bytes)?;
        let compressed = doc.header.compressed();

        // Pass 2: everything else.
        let mut stream_paths: Vec<String> = comp
            .walk()
            .filter(|e| e.is_stream())
            .map(|e| e.path().to_string_lossy().into_owned())
            .filter(|p| p != streams::file_header::STREAM_NAME)
            .collect();

        // Stable ordering for /BodyText/Section{N}: numeric, not lexical.
        stream_paths.sort_by(|a, b| {
            let a_idx = streams::body_text::section_index_of(a);
            let b_idx = streams::body_text::section_index_of(b);
            match (a_idx, b_idx) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Greater,
                (None, Some(_)) => std::cmp::Ordering::Less,
                (None, None) => a.cmp(b),
            }
        });

        for path in stream_paths {
            let bytes = read_stream(&mut comp, &path)?;

            if path == streams::doc_info::STREAM_NAME {
                let records = if compressed {
                    streams::doc_info::parse(&bytes)?
                } else {
                    streams::record::parse_all(&bytes)?
                };
                doc.doc_info.raw_records = records;
                doc.doc_info.stream_bytes = Some(bytes);
                populate_typed_views(&mut doc.doc_info)?;
                continue;
            }

            if streams::body_text::section_index_of(&path).is_some() {
                let section = streams::body_text::parse_section(&bytes, compressed)?;
                doc.sections.push(section);
                continue;
            }

            doc.unknown_streams.insert(path, bytes);
        }

        Ok(doc)
    }
}

/// Crack open the DocInfo record stream into typed IR views. Kept separate
/// from record parsing so it can be re-run after mutations without re-reading.
fn populate_typed_views(info: &mut DocInfo) -> Result<(), IrError> {
    populate_document_properties(info)?;
    populate_font_faces(info)?;
    populate_border_fills(info)?;
    populate_char_shapes(info)?;
    populate_para_shapes(info)?;
    populate_styles(info)?;
    Ok(())
}

fn populate_document_properties(info: &mut DocInfo) -> Result<(), IrError> {
    if let Some(rec) = info
        .raw_records
        .iter()
        .find(|r| r.tag == streams::doc_info::tag::DOCUMENT_PROPERTIES)
    {
        info.properties = streams::document_properties::parse(rec)?;
    }
    Ok(())
}

fn populate_border_fills(info: &mut DocInfo) -> Result<(), IrError> {
    info.border_fills = info
        .raw_records
        .iter()
        .filter(|r| r.tag == streams::doc_info::tag::BORDER_FILL)
        .map(streams::border_fill::parse)
        .collect::<Result<_, _>>()?;
    Ok(())
}

fn populate_char_shapes(info: &mut DocInfo) -> Result<(), IrError> {
    info.char_shapes = info
        .raw_records
        .iter()
        .filter(|r| r.tag == streams::doc_info::tag::CHAR_SHAPE)
        .map(streams::char_shape::parse)
        .collect::<Result<_, _>>()?;
    Ok(())
}

fn populate_para_shapes(info: &mut DocInfo) -> Result<(), IrError> {
    info.para_shapes = info
        .raw_records
        .iter()
        .filter(|r| r.tag == streams::doc_info::tag::PARA_SHAPE)
        .map(streams::para_shape::parse)
        .collect::<Result<_, _>>()?;
    Ok(())
}

fn populate_styles(info: &mut DocInfo) -> Result<(), IrError> {
    info.styles = info
        .raw_records
        .iter()
        .filter(|r| r.tag == streams::doc_info::tag::STYLE)
        .map(streams::style::parse)
        .collect::<Result<_, _>>()?;
    Ok(())
}

fn populate_font_faces(info: &mut DocInfo) -> Result<(), IrError> {
    let id_map = info
        .raw_records
        .iter()
        .find(|r| r.tag == streams::doc_info::tag::ID_MAPPINGS)
        .map(streams::id_mappings::parse)
        .transpose()?;
    let Some(id_map) = id_map else { return Ok(()); };

    let mut face_iter = info
        .raw_records
        .iter()
        .filter(|r| r.tag == streams::doc_info::tag::FACE_NAME);

    let take = |iter: &mut dyn Iterator<Item = &hwp_transpiler_core::ir::Record>,
                n: u32,
                slot: &str|
     -> Result<Vec<FontFace>, IrError> {
        (0..n)
            .map(|i| {
                let r = iter.next().ok_or_else(|| {
                    IrError::Invalid(format!(
                        "IdMappings claims {n} faces in slot '{slot}' but raw_records exhausted at index {i}"
                    ))
                })?;
                streams::face_name::parse(r)
            })
            .collect()
    };

    info.font_faces.hangul = take(&mut face_iter, id_map.fonts[0], "hangul")?;
    info.font_faces.latin = take(&mut face_iter, id_map.fonts[1], "latin")?;
    info.font_faces.hanja = take(&mut face_iter, id_map.fonts[2], "hanja")?;
    info.font_faces.japanese = take(&mut face_iter, id_map.fonts[3], "japanese")?;
    info.font_faces.other = take(&mut face_iter, id_map.fonts[4], "other")?;
    info.font_faces.symbol = take(&mut face_iter, id_map.fonts[5], "symbol")?;
    info.font_faces.user = take(&mut face_iter, id_map.fonts[6], "user")?;
    Ok(())
}

fn read_stream(
    comp: &mut cfb::CompoundFile<Cursor<Vec<u8>>>,
    path: &str,
) -> Result<Vec<u8>, IrError> {
    let mut s = comp
        .open_stream(path)
        .map_err(|e| IrError::Invalid(format!("open stream {path}: {e}")))?;
    let mut bytes = Vec::with_capacity(s.len() as usize);
    s.read_to_end(&mut bytes)?;
    Ok(bytes)
}
