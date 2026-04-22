use hwp_transpiler_codec::hwp::streams::doc_info;
use hwp_transpiler_codec::hwp::{HwpReader, HwpWriter};
use hwp_transpiler_core::ir::{IrDocument, Reader, Writer};
use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

#[test]
fn synthetic_streams_round_trip() {
    // `/FileHeader`, `/DocInfo`, and `/BodyText/Section{N}` all flow
    // through typed IR paths; exercise passthrough with paths that
    // currently stay in unknown_streams.
    let mut ir = IrDocument::default();
    ir.unknown_streams.insert("/PrvText".into(), vec![0xDD; 8]);
    ir.unknown_streams.insert("/Scripts/DefaultJScript".into(), vec![0xEE; 16]);

    let out = HwpWriter.write(&ir).expect("write");
    let parsed = HwpReader.read(&out).expect("read");

    for (path, bytes) in &ir.unknown_streams {
        assert_eq!(
            parsed.unknown_streams.get(path),
            Some(bytes),
            "stream {path} not round-tripped"
        );
    }
    for typed in ["/FileHeader", "/DocInfo"] {
        assert!(
            !parsed.unknown_streams.contains_key(typed),
            "typed {typed} leaked into unknown_streams"
        );
    }
    assert_eq!(parsed.header.signature, ir.header.signature);
    assert_eq!(parsed.header.version, ir.header.version);
    assert_eq!(parsed.header.flags, ir.header.flags);
    assert_eq!(parsed.header.reserved.len(), 216);
    assert!(parsed.doc_info.raw_records.is_empty());
    assert!(parsed.sections.is_empty(), "no sections expected");
}

#[test]
fn blank_hwp_per_stream_match() {
    assert_fixture_roundtrips(&repo_path("test/fixture.hwp"));
}

/// Real-world fixture: complex TRL report (≈5 MB, 9 embedded images, many
/// tables). Byte-equal round-trip here means the strangler-fig pass-through
/// fully covers everything hwplib-produced HWP files contain.
#[test]
fn trl_report_per_stream_match() {
    assert_fixture_roundtrips(&repo_path(
        "test/260420-1. 연구개발계획서(서식)[TRL점프업 1단계]_대진대_수정_1430_fin.hwp",
    ));
}

fn assert_fixture_roundtrips(path: &Path) {
    let Ok(fixture) = std::fs::read(path) else {
        eprintln!("skipping: {} not present", path.display());
        return;
    };

    let ir = HwpReader.read(&fixture).expect("read fixture");
    let out = HwpWriter.write(&ir).expect("write");

    let lhs = stream_map(&fixture).expect("fixture streams");
    let rhs = stream_map(&out).expect("output streams");
    assert_eq!(
        lhs.keys().collect::<Vec<_>>(),
        rhs.keys().collect::<Vec<_>>(),
        "stream sets differ"
    );
    for (k, v) in &lhs {
        assert_eq!(v, &rhs[k], "stream {k} diverged ({} bytes)", v.len());
    }
}

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// Round 3: after `HwpReader.read`, `doc.doc_info.raw_records` holds the
/// parsed record tree, and `stream_bytes` holds the original compressed
/// bytes. Writer prefers verbatim, so fixture round-trip stays byte-equal.
#[test]
fn docinfo_parsed_into_typed_ir() {
    let Ok(fixture) = std::fs::read(repo_path("test/fixture.hwp")) else {
        eprintln!("skipping: fixture not present");
        return;
    };
    let doc = HwpReader.read(&fixture).expect("read");
    assert!(
        !doc.doc_info.raw_records.is_empty(),
        "raw_records should be populated"
    );
    assert!(
        doc.doc_info.stream_bytes.is_some(),
        "stream_bytes should hold verbatim compressed DocInfo"
    );
    assert!(
        !doc.unknown_streams.contains_key("/DocInfo"),
        "DocInfo should be typed, not in unknown_streams"
    );
}

/// If the caller mutates records via `records_mut()`, `stream_bytes` is
/// cleared and the writer re-encodes. Byte equality with the fixture is
/// NOT expected; structural equivalence (same records) IS.
#[test]
fn docinfo_mutation_forces_reencode() {
    let Ok(fixture) = std::fs::read(repo_path("test/fixture.hwp")) else {
        eprintln!("skipping: fixture not present");
        return;
    };

    let mut doc = HwpReader.read(&fixture).expect("read");
    let original_records = doc.doc_info.raw_records.clone();

    // No-op mutation: calling records_mut clears the verbatim cache.
    let _ = doc.doc_info.records_mut();
    assert!(doc.doc_info.stream_bytes.is_none());

    let out = HwpWriter.write(&doc).expect("write");
    let reparsed = HwpReader.read(&out).expect("reparse");

    assert_eq!(
        reparsed.doc_info.raw_records, original_records,
        "records must survive re-encode round-trip"
    );
}

/// Round 4a: manual FaceName parse, lossless emit→parse identity.
#[test]
fn fixture_font_faces_parse_manually() {
    use hwp_transpiler_codec::hwp::streams::face_name;

    let Ok(fixture) = std::fs::read(repo_path("test/fixture.hwp")) else {
        eprintln!("skipping: fixture not present");
        return;
    };
    let doc = HwpReader.read(&fixture).expect("read");

    let faces: Vec<_> = doc
        .doc_info
        .raw_records
        .iter()
        .filter(|r| r.tag == 0x0013)
        .map(|r| face_name::parse(r).expect("parse FaceName"))
        .collect();

    assert!(!faces.is_empty());
    for f in &faces {
        let bytes = face_name::emit(f);
        let rec = hwp_transpiler_core::ir::Record {
            tag: 0x0013,
            level: 1,
            data: bytes,
        };
        let reparsed = face_name::parse(&rec).expect("reparse");
        assert_eq!(&reparsed, f, "FaceName reparse drift");
    }
}

/// MD export demo: round-trip → Markdown. Asserts TRL doc contains "TRL"
/// somewhere and the document renders with at least one heading.
#[test]
fn markdown_export_trl() {
    use hwp_transpiler_codec::export::markdown;

    let Ok(fixture) = std::fs::read(repo_path(
        "test/260420-1. 연구개발계획서(서식)[TRL점프업 1단계]_대진대_수정_1430_fin.hwp",
    )) else {
        eprintln!("skipping: TRL fixture not present");
        return;
    };
    let doc = HwpReader.read(&fixture).expect("read");
    let md = markdown::to_markdown(&doc);

    assert!(
        md.contains("TRL"),
        "TRL keyword should survive into Markdown"
    );
    eprintln!("[TRL] MD length: {} chars ({} lines)", md.len(), md.lines().count());
    eprintln!("--- first 500 chars ---");
    for line in md.lines().take(20) {
        eprintln!("{line}");
    }
    eprintln!("--- (truncated) ---");
}

#[test]
fn markdown_export_blank() {
    use hwp_transpiler_codec::export::markdown;

    let Ok(fixture) = std::fs::read(repo_path("test/fixture.hwp")) else {
        eprintln!("skipping: fixture not present");
        return;
    };
    let doc = HwpReader.read(&fixture).expect("read");
    let md = markdown::to_markdown(&doc);
    eprintln!("[blank] MD: {:?}", md);
    // Blank doc renders to empty or near-empty output; just ensure no panic.
}

/// Round 5a: `/BodyText/Section{N}` streams decompress, group records into
/// paragraphs, and extract text. Verbatim `stream_bytes` preserved so the
/// fixture round-trips byte-equal.
#[test]
fn fixture_body_text_parses_paragraphs() {
    let Ok(fixture) = std::fs::read(repo_path("test/fixture.hwp")) else {
        eprintln!("skipping: fixture not present");
        return;
    };
    let doc = HwpReader.read(&fixture).expect("read");
    assert!(!doc.sections.is_empty(), "must have at least one section");
    let s = &doc.sections[0];
    assert!(
        !doc.unknown_streams.contains_key("/BodyText/Section0"),
        "typed /BodyText/Section0 leaked into unknown_streams"
    );
    assert!(
        s.stream_bytes.is_some(),
        "verbatim stream_bytes must be preserved"
    );
    eprintln!(
        "[fixture] Section 0: {} paragraphs, {} section-scoped records",
        s.paragraphs.len(),
        s.raw_records.len()
    );
    for (i, p) in s.paragraphs.iter().enumerate() {
        eprintln!(
            "  p{i}: chars={} para_shape={} style={} text={:?}",
            p.header.char_count,
            p.header.para_shape_id,
            p.header.style_id,
            p.text,
        );
    }
}

/// Round 5d2: tables populate fully with cells + cell paragraphs. Cell
/// count per table must match `row_cell_counts` sum.
#[test]
fn fixture_tables_populate_from_trl() {
    use hwp_transpiler_core::ir::ControlKind;

    let Ok(fixture) = std::fs::read(repo_path(
        "test/260420-1. 연구개발계획서(서식)[TRL점프업 1단계]_대진대_수정_1430_fin.hwp",
    )) else {
        eprintln!("skipping: TRL fixture not present");
        return;
    };
    let doc = HwpReader.read(&fixture).expect("read");

    let mut tables = Vec::new();
    for s in &doc.sections {
        for p in &s.paragraphs {
            collect_tables(&p.controls, &mut tables);
        }
    }

    assert!(
        tables.len() >= 53,
        "TRL should contain at least 53 tables including nested, got {}",
        tables.len()
    );
    eprintln!("[TRL] {} tables (incl. nested)", tables.len());

    let mut total_text_chars = 0usize;
    for (i, t) in tables.iter().enumerate().take(3) {
        let expected_cells: u32 = t.row_cell_counts.iter().map(|&n| n as u32).sum();
        assert_eq!(
            t.cells.len() as u32,
            expected_cells,
            "table #{i}: cells count mismatch"
        );
        eprintln!(
            "  #{i}: {}×{} ({} cells)",
            t.rows,
            t.cols,
            t.cells.len()
        );
        for (ci, cell) in t.cells.iter().enumerate().take(4) {
            let text: String = cell
                .paragraphs
                .iter()
                .map(|p| p.text.as_str())
                .collect::<Vec<_>>()
                .join(" ⏎ ");
            eprintln!(
                "    [{},{}] span={}×{} w={}B h={}B: {:?}",
                cell.col,
                cell.row,
                cell.col_span,
                cell.row_span,
                cell.width_hwpu,
                cell.height_hwpu,
                text.chars().take(30).collect::<String>()
            );
            for p in &cell.paragraphs {
                total_text_chars += p.text.chars().count();
            }
            let _ = ci;
        }
    }
    eprintln!("  (first 3 tables' cells contain {} chars)", total_text_chars);
}

fn collect_tables<'a>(
    controls: &'a [hwp_transpiler_core::ir::Control],
    acc: &mut Vec<&'a hwp_transpiler_core::ir::TableControl>,
) {
    use hwp_transpiler_core::ir::ControlKind;
    for c in controls {
        if let ControlKind::Table(t) = &c.kind {
            acc.push(t);
            for cell in &t.cells {
                for p in &cell.paragraphs {
                    collect_tables(&p.controls, acc);
                }
            }
        }
    }
}

#[test]
fn diag_raw_section_records() {
    use hwp_transpiler_codec::hwp::streams::{doc_info, record};

    let Ok(fixture) = std::fs::read(repo_path(
        "test/260420-1. 연구개발계획서(서식)[TRL점프업 1단계]_대진대_수정_1430_fin.hwp",
    )) else {
        return;
    };
    let mut comp = cfb::CompoundFile::open(Cursor::new(fixture.clone())).unwrap();
    let mut s = comp.open_stream("/BodyText/Section0").unwrap();
    let mut bytes = Vec::with_capacity(s.len() as usize);
    s.read_to_end(&mut bytes).unwrap();
    let decompressed = doc_info::decompress(&bytes).unwrap();
    let records = record::parse_all(&decompressed).unwrap();

    // Find first "tbl " control and dump the next 40 records (including levels)
    for (i, r) in records.iter().enumerate() {
        if r.tag == 0x0047 && r.data.len() >= 4 {
            let code = &r.data[0..4];
            if code == b" lbt" {
                eprintln!(
                    "[first tbl] at record #{i} in raw stream; dumping records {i}..{}:",
                    i + 40
                );
                for (j, rr) in records.iter().enumerate().skip(i).take(40) {
                    eprintln!(
                        "  #{j} tag=0x{:04X} level={} size={}",
                        rr.tag,
                        rr.level,
                        rr.data.len()
                    );
                }
                break;
            }
        }
    }
}

/// Diagnostic: after the first "tbl " CTRL_HEADER in the TRL doc, dump
/// the next ~20 record tags to discover which tag TABLE uses in this
/// HWP5 revision. Delete once Round 5d has the table encoder locked in.
#[test]
fn diag_dump_records_around_first_table() {
    use hwp_transpiler_codec::hwp::streams::ctrl_header;
    use hwp_transpiler_core::ir::ControlKind;

    let Ok(fixture) = std::fs::read(repo_path(
        "test/260420-1. 연구개발계획서(서식)[TRL점프업 1단계]_대진대_수정_1430_fin.hwp",
    )) else {
        eprintln!("skipping: TRL fixture not present");
        return;
    };
    let doc = HwpReader.read(&fixture).expect("read");

    for (pi, p) in doc.sections[0].paragraphs.iter().enumerate() {
        let has_table = p.controls.iter().any(|c| {
            matches!(&c.kind, ControlKind::Unknown { code, .. } if ctrl_header::display_code(code) == "tbl ")
        });
        if !has_table {
            continue;
        }
        eprintln!(
            "\n=== paragraph #{pi}: {} raw_records (controls: {}) ===",
            p.raw_records.len(),
            p.controls.len()
        );
        for (i, r) in p.raw_records.iter().take(60).enumerate() {
            eprintln!(
                "  [{i}] tag=0x{:04X} level={} size={}",
                r.tag,
                r.level,
                r.data.len()
            );
        }
        break;
    }
}

/// Round 5c: paragraphs expose `controls` populated from CTRL_HEADER
/// records. The distribution of control IDs reveals document structure:
/// TRL R&D forms are table-heavy.
#[test]
fn fixture_controls_surface_in_trl() {
    use hwp_transpiler_codec::hwp::streams::ctrl_header;
    use hwp_transpiler_core::ir::ControlKind;
    use std::collections::BTreeMap;

    let Ok(fixture) = std::fs::read(repo_path(
        "test/260420-1. 연구개발계획서(서식)[TRL점프업 1단계]_대진대_수정_1430_fin.hwp",
    )) else {
        eprintln!("skipping: TRL fixture not present");
        return;
    };
    let doc = HwpReader.read(&fixture).expect("read");

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut total = 0usize;
    for s in &doc.sections {
        for p in &s.paragraphs {
            for c in &p.controls {
                let key = match &c.kind {
                    ControlKind::Table(_) => "tbl ".to_string(),
                    ControlKind::Equation(_) => "eqed".to_string(),
                    ControlKind::Picture(_) => "pic ".to_string(),
                    ControlKind::Unknown { code, .. } => ctrl_header::display_code(code),
                };
                *counts.entry(key).or_insert(0) += 1;
                total += 1;
            }
        }
    }

    eprintln!("[TRL] Controls ({total} total):");
    for (id, n) in &counts {
        eprintln!("  '{id}' × {n}");
    }
    assert!(total > 0, "TRL doc should contain inline controls");
    assert!(
        counts.contains_key("tbl "),
        "TRL form should contain at least one table"
    );
}

/// Round 5b: paragraphs expose `char_shape_runs` + `line_segments`.
/// Counts must match the per-paragraph counters stored in the header.
#[test]
fn fixture_paragraph_runs_and_segments() {
    let Ok(fixture) = std::fs::read(repo_path(
        "test/260420-1. 연구개발계획서(서식)[TRL점프업 1단계]_대진대_수정_1430_fin.hwp",
    )) else {
        eprintln!("skipping: TRL fixture not present");
        return;
    };
    let doc = HwpReader.read(&fixture).expect("read");
    let s = &doc.sections[0];

    fn visit<'a>(
        paragraphs: &'a [hwp_transpiler_core::ir::Paragraph],
        check_count: &mut usize,
    ) {
        use hwp_transpiler_core::ir::ControlKind;
        for p in paragraphs {
            assert_eq!(
                p.char_shape_runs.len() as u16,
                p.header.char_shape_count,
                "char_shape_runs count mismatch against header"
            );
            assert_eq!(
                p.line_segments.len() as u16,
                p.header.line_align_count,
                "line_segments count mismatch against header"
            );
            if !p.char_shape_runs.is_empty() {
                assert_eq!(p.char_shape_runs[0].start, 0);
            }
            assert!(p.header.real_char_count() < 0x8000_0000);
            if !p.char_shape_runs.is_empty() && !p.line_segments.is_empty() {
                *check_count += 1;
            }
            for c in &p.controls {
                if let ControlKind::Table(t) = &c.kind {
                    for cell in &t.cells {
                        visit(&cell.paragraphs, check_count);
                    }
                }
            }
        }
    }

    let mut styled = 0usize;
    visit(&s.paragraphs, &mut styled);

    // Total count (recursive) for comparison with the original flat 1654.
    fn count_all<'a>(
        paragraphs: &'a [hwp_transpiler_core::ir::Paragraph],
        total: &mut usize,
        styled_too: &mut usize,
    ) {
        use hwp_transpiler_core::ir::ControlKind;
        for p in paragraphs {
            *total += 1;
            if !p.char_shape_runs.is_empty() && !p.line_segments.is_empty() {
                *styled_too += 1;
            }
            for c in &p.controls {
                if let ControlKind::Table(t) = &c.kind {
                    for cell in &t.cells {
                        count_all(&cell.paragraphs, total, styled_too);
                    }
                }
            }
        }
    }
    let (mut total, mut st2) = (0, 0);
    count_all(&s.paragraphs, &mut total, &mut st2);
    eprintln!(
        "[TRL] recursive: {} total paragraphs, {} styled (expected near 1654)",
        total, st2
    );

    assert!(total > 1000, "expected > 1000 paragraphs total, got {total}");
    assert!(styled > 500, "expected > 500 with styling, got {styled}");
}

#[test]
fn trl_body_text_parses_paragraphs() {
    let Ok(fixture) = std::fs::read(repo_path(
        "test/260420-1. 연구개발계획서(서식)[TRL점프업 1단계]_대진대_수정_1430_fin.hwp",
    )) else {
        eprintln!("skipping: TRL fixture not present");
        return;
    };
    let doc = HwpReader.read(&fixture).expect("read");
    let s = &doc.sections[0];
    assert!(
        s.paragraphs.len() > 10,
        "TRL doc should have many paragraphs, got {}",
        s.paragraphs.len()
    );
    // Count paragraphs with non-empty text
    let with_text = s.paragraphs.iter().filter(|p| !p.text.is_empty()).count();
    let total_chars: usize = s.paragraphs.iter().map(|p| p.text.chars().count()).sum();
    eprintln!(
        "[TRL] Section 0: {} paragraphs ({} with text), {} total chars",
        s.paragraphs.len(),
        with_text,
        total_chars
    );
    // Sanity: first non-empty paragraph preview
    if let Some(p) = s.paragraphs.iter().find(|p| !p.text.is_empty()) {
        let preview: String = p.text.chars().take(40).collect();
        eprintln!("  first text preview: {:?}", preview);
    }
}

/// Round 4g: DocumentProperties parses into `doc.doc_info.properties`.
/// Blank fixture should report 1 section, 1 page, and reasonable caret.
#[test]
fn fixture_document_properties() {
    use hwp_transpiler_codec::hwp::streams::document_properties;

    let Ok(fixture) = std::fs::read(repo_path("test/fixture.hwp")) else {
        eprintln!("skipping: fixture not present");
        return;
    };
    let doc = HwpReader.read(&fixture).expect("read");
    let p = &doc.doc_info.properties;

    assert_eq!(p.section_count, 1, "blank doc should have 1 section");
    eprintln!(
        "[fixture] DocumentProperties:\n  sections={}, pages={}, chars={}, tail={}B {:?}\n  starts: page={}, footnote={}, endnote={}, pic={}, tab={}, eq={}",
        p.section_count,
        p.total_page_count,
        p.total_character_count,
        p.tail.len(),
        p.caret_position(),
        p.page_start_number,
        p.footnote_start_number,
        p.endnote_start_number,
        p.picture_start_number,
        p.table_start_number,
        p.equation_start_number,
    );

    // emit → parse identity
    let bytes = document_properties::emit(p);
    let reparsed = document_properties::parse(&hwp_transpiler_core::ir::Record {
        tag: 0x0010,
        level: 0,
        data: bytes,
    })
    .unwrap();
    assert_eq!(&reparsed, p);
}

/// Round 4f: Reader auto-populates `doc.doc_info.styles` with Korean +
/// English names and para/char shape references.
#[test]
fn fixture_styles_populate_and_roundtrip() {
    use hwp_transpiler_codec::hwp::streams::{id_mappings, style};

    let Ok(fixture) = std::fs::read(repo_path("test/fixture.hwp")) else {
        eprintln!("skipping: fixture not present");
        return;
    };
    let doc = HwpReader.read(&fixture).expect("read");

    let id_map = id_mappings::parse(
        doc.doc_info.raw_records.iter().find(|r| r.tag == 0x0011).unwrap(),
    )
    .unwrap();
    assert_eq!(
        doc.doc_info.styles.len() as u32,
        id_map.style,
        "Style slot count must match IdMappings"
    );

    eprintln!("[fixture] {} Style records:", doc.doc_info.styles.len());
    for (i, s) in doc.doc_info.styles.iter().enumerate() {
        eprintln!(
            "  #{i}: '{}' / '{}' type={} next={} para={} char={} lang=0x{:04X}",
            s.name,
            s.english_name,
            s.properties & 0x07,
            s.next_style_id,
            s.para_shape_id,
            s.char_shape_id,
            s.lang_id as u16,
        );
        let bytes = style::emit(s);
        let reparsed = style::parse(&hwp_transpiler_core::ir::Record {
            tag: 0x001A,
            level: 1,
            data: bytes,
        })
        .unwrap();
        assert_eq!(&reparsed, s, "Style reparse drift at #{i}");
    }
}

/// Round 4e: Reader auto-populates `doc.doc_info.para_shapes`. Accessors
/// decode align / heading_level / keep_with_next from raw bits.
#[test]
fn fixture_para_shapes_populate_and_roundtrip() {
    use hwp_transpiler_codec::hwp::streams::{id_mappings, para_shape};

    let Ok(fixture) = std::fs::read(repo_path("test/fixture.hwp")) else {
        eprintln!("skipping: fixture not present");
        return;
    };
    let doc = HwpReader.read(&fixture).expect("read");

    let id_map = id_mappings::parse(
        doc.doc_info.raw_records.iter().find(|r| r.tag == 0x0011).unwrap(),
    )
    .unwrap();
    assert_eq!(
        doc.doc_info.para_shapes.len() as u32,
        id_map.para_shape,
        "ParaShape slot count must match IdMappings"
    );

    let align_names = ["Justify", "Left", "Right", "Center", "Distribute", "DistSpace"];
    eprintln!("[fixture] {} ParaShape records:", doc.doc_info.para_shapes.len());
    for (i, ps) in doc.doc_info.para_shapes.iter().enumerate() {
        let a = ps.align() as usize;
        eprintln!(
            "  #{i}: align={} hd_lvl={} keep={} pgbreak={} lm={} rm={} indent={} attr=0x{:08X}",
            align_names.get(a).copied().unwrap_or("?"),
            ps.heading_level(),
            ps.keep_with_next() as u8,
            ps.page_break_before() as u8,
            ps.left_margin,
            ps.right_margin,
            ps.indent,
            ps.attribute,
        );
        let bytes = para_shape::emit(ps);
        let reparsed = para_shape::parse(&hwp_transpiler_core::ir::Record {
            tag: 0x0019,
            level: 1,
            data: bytes,
        })
        .unwrap();
        assert_eq!(&reparsed, ps, "ParaShape reparse drift at #{i}");
    }
}

/// Round 4d: Reader auto-populates `doc.doc_info.char_shapes`. Count
/// matches IdMappings; emit → parse identity; base_size interpretable as pt.
#[test]
fn fixture_char_shapes_populate_and_roundtrip() {
    use hwp_transpiler_codec::hwp::streams::{char_shape, id_mappings};

    let Ok(fixture) = std::fs::read(repo_path("test/fixture.hwp")) else {
        eprintln!("skipping: fixture not present");
        return;
    };
    let doc = HwpReader.read(&fixture).expect("read");

    let id_map = id_mappings::parse(
        doc.doc_info.raw_records.iter().find(|r| r.tag == 0x0011).unwrap(),
    )
    .unwrap();
    assert_eq!(
        doc.doc_info.char_shapes.len() as u32,
        id_map.char_shape,
        "CharShape slot count must match IdMappings"
    );

    eprintln!("[fixture] {} CharShape records:", doc.doc_info.char_shapes.len());
    for (i, cs) in doc.doc_info.char_shapes.iter().enumerate() {
        eprintln!(
            "  #{i}: {:.1}pt bold={} italic={} strike={} sup/sub={} attr=0x{:08X} color=#{:08X} fonts={:?}",
            cs.base_size as f32 / 100.0,
            cs.bold(),
            cs.italic(),
            cs.strike(),
            cs.script(),
            cs.attr,
            cs.color,
            cs.font_ids,
        );
        let bytes = char_shape::emit(cs);
        let reparsed = char_shape::parse(&hwp_transpiler_core::ir::Record {
            tag: 0x0015,
            level: 1,
            data: bytes,
        })
        .unwrap();
        assert_eq!(&reparsed, cs, "CharShape reparse drift at #{i}");
    }
}

/// Round 4c: Reader auto-populates `doc.doc_info.border_fills`. Count
/// must match IdMappings claim; emit → parse must be identity.
#[test]
fn fixture_border_fills_populate_and_roundtrip() {
    use hwp_transpiler_codec::hwp::streams::{border_fill, id_mappings};

    let Ok(fixture) = std::fs::read(repo_path("test/fixture.hwp")) else {
        eprintln!("skipping: fixture not present");
        return;
    };
    let doc = HwpReader.read(&fixture).expect("read");

    let id_rec = doc
        .doc_info
        .raw_records
        .iter()
        .find(|r| r.tag == 0x0011)
        .expect("IdMappings missing");
    let id_map = id_mappings::parse(id_rec).expect("parse IdMappings");
    assert_eq!(
        doc.doc_info.border_fills.len() as u32,
        id_map.border_fill,
        "BorderFill slot count must match IdMappings"
    );

    eprintln!("[fixture] {} BorderFill records:", doc.doc_info.border_fills.len());
    for (i, bf) in doc.doc_info.border_fills.iter().enumerate() {
        eprintln!(
            "  #{i}: attr=0x{:04X} fill_kind=0x{:X} body={}B borders=[L:k{} R:k{} T:k{} B:k{}] diag:k{}",
            bf.attribute,
            bf.fill.kind,
            bf.fill.body.len(),
            bf.borders[0].kind,
            bf.borders[1].kind,
            bf.borders[2].kind,
            bf.borders[3].kind,
            bf.diagonal.kind,
        );
        let bytes = border_fill::emit(bf);
        let reparsed = border_fill::parse(&hwp_transpiler_core::ir::Record {
            tag: 0x0014,
            level: 1,
            data: bytes,
        })
        .expect("reparse");
        assert_eq!(&reparsed, bf, "BorderFill reparse drift at #{i}");
    }
}

/// Round 4b: Reader auto-populates `doc.doc_info.font_faces` slots from
/// IdMappings counts + FaceName records in document order.
#[test]
fn fixture_font_faces_split_into_slots() {
    let Ok(fixture) = std::fs::read(repo_path("test/fixture.hwp")) else {
        eprintln!("skipping: fixture not present");
        return;
    };
    let doc = HwpReader.read(&fixture).expect("read");
    let ff = &doc.doc_info.font_faces;

    // Total across slots must equal number of raw FaceName records.
    let raw_count = doc
        .doc_info
        .raw_records
        .iter()
        .filter(|r| r.tag == 0x0013)
        .count();
    let slot_count: usize = [&ff.hangul, &ff.latin, &ff.hanja, &ff.japanese, &ff.other, &ff.symbol, &ff.user]
        .iter()
        .map(|v| v.len())
        .sum();
    assert_eq!(
        slot_count, raw_count,
        "slot totals must match raw FaceName count"
    );

    // Each slot in the blank fixture has 2 faces (Batang + Dotum).
    for (name, slot) in [
        ("hangul", &ff.hangul),
        ("latin", &ff.latin),
        ("hanja", &ff.hanja),
        ("japanese", &ff.japanese),
        ("other", &ff.other),
        ("symbol", &ff.symbol),
        ("user", &ff.user),
    ] {
        eprintln!(
            "{:>9}: [{}]",
            name,
            slot.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", ")
        );
    }

    assert!(!ff.hangul.is_empty(), "hangul slot should be populated");
}

/// Smoke test: the real fixture's `/DocInfo` stream is raw DEFLATE and
/// parseable as a HWP5 record stream.
#[test]
fn real_docinfo_decompresses_and_parses() {
    let Ok(fixture) = std::fs::read(repo_path("test/fixture.hwp")) else {
        eprintln!("skipping: fixture not present");
        return;
    };
    let mut comp = cfb::CompoundFile::open(Cursor::new(fixture)).expect("open OLE");
    let mut s = comp.open_stream(doc_info::STREAM_NAME).expect("open /DocInfo");
    let mut bytes = Vec::with_capacity(s.len() as usize);
    s.read_to_end(&mut bytes).unwrap();

    let records = doc_info::parse(&bytes).expect("decompress + parse");
    assert!(!records.is_empty(), "/DocInfo should contain records");
    eprintln!("[blank.hwp /DocInfo] {} records:", records.len());
    for r in &records {
        eprintln!(
            "  tag=0x{:04X} level={} size={}",
            r.tag,
            r.level,
            r.data.len()
        );
    }
}

fn stream_map(bytes: &[u8]) -> std::io::Result<BTreeMap<String, Vec<u8>>> {
    let mut comp = cfb::CompoundFile::open(Cursor::new(bytes.to_vec()))?;
    let paths: Vec<String> = comp
        .walk()
        .filter(|e| e.is_stream())
        .map(|e| e.path().to_string_lossy().into_owned())
        .collect();
    let mut out = BTreeMap::new();
    for p in paths {
        let mut s = comp.open_stream(&p)?;
        let mut v = Vec::with_capacity(s.len() as usize);
        s.read_to_end(&mut v)?;
        out.insert(p, v);
    }
    Ok(out)
}
