//! L1 skeleton integration: run `to_markdown_with` in LLM mode on the
//! TRL fixture, verify that every emitted id is unique and that known
//! captions survive into the structured output.

use hwp_transpiler_codec::export::markdown;
use hwp_transpiler_codec::hwp::HwpReader;
use hwp_transpiler_core::ir::Reader;
use std::collections::HashSet;
use std::path::Path;

fn repo_path(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(rel)
}

/// Pull every `id=<value>` attribute that appears inside a `FOO[id=...]`
/// header. Only the first comma-terminated token after `id=` on each
/// matching line counts.
fn collect_ids(md: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in md.lines() {
        if let Some(pos) = line.find("[id=") {
            let rest = &line[pos + 4..];
            let end = rest
                .find(|c: char| c == ',' || c == ']')
                .unwrap_or(rest.len());
            out.push(rest[..end].to_string());
        }
    }
    out
}

#[test]
fn trl_llm_output_has_globally_unique_ids() {
    let fixture = repo_path(
        "test/260420-1. 연구개발계획서(서식)[TRL점프업 1단계]_대진대_수정_1430_fin.hwp",
    );
    let Ok(bytes) = std::fs::read(&fixture) else {
        eprintln!("skipping: TRL fixture not present");
        return;
    };
    let doc = HwpReader.read(&bytes).expect("read");
    let md = markdown::to_markdown_with(
        &doc,
        &markdown::MdOptions {
            llm: Some(markdown::LlmOptions::default()),
            ..markdown::MdOptions::default()
        },
    );

    // The whole point of the positional id scheme is global uniqueness;
    // if HashSet::insert returns false the scheme is broken for real
    // documents.
    let ids = collect_ids(&md);
    let mut seen = HashSet::new();
    for id in &ids {
        assert!(
            seen.insert(id.clone()),
            "duplicate id {id} in LLM markdown output"
        );
    }
    assert!(ids.len() > 100, "TRL fixture should yield ≥100 ids, got {}", ids.len());

    // Structural expectations: we should see at least one figure, caption,
    // table, and cell marker.
    assert!(md.contains("FIGURE[id=fig-"), "no FIGURE marker: first 200b: {}", &md[..md.len().min(200)]);
    assert!(md.contains("CAPTION[id=cap-fig-"), "no CAPTION marker");
    assert!(md.contains("TABLE[id=tbl-s0-"), "no TABLE marker");
    assert!(md.contains("CELL[id=cell-s0-"), "no CELL marker");

    // Known captions must show up.
    assert!(
        md.contains("시스템 전체 아키텍처") || md.contains("제품 라인업"),
        "no recognisable caption text in LLM output"
    );

    // No FFFC leakage.
    assert!(
        !md.contains('\u{FFFC}'),
        "raw FFFC must not leak into LLM output"
    );
}

#[test]
fn trl_llm_with_roles_produces_mixed_distribution() {
    // L2 integration: emit_roles flag must surface Header/Label/Content
    // from the visual classifier, not all the same role. TRL's Korean-
    // government layout yields a clear majority of values with a
    // meaningful header row and some coloured label cells.
    let fixture = repo_path(
        "test/260420-1. 연구개발계획서(서식)[TRL점프업 1단계]_대진대_수정_1430_fin.hwp",
    );
    let Ok(bytes) = std::fs::read(&fixture) else {
        eprintln!("skipping: TRL fixture not present");
        return;
    };
    let doc = HwpReader.read(&bytes).expect("read");
    let md = markdown::to_markdown_with(
        &doc,
        &markdown::MdOptions {
            llm: Some(markdown::LlmOptions {
                emit_roles: true,
                ..markdown::LlmOptions::default()
            }),
            ..markdown::MdOptions::default()
        },
    );

    let headers = md.match_indices("role=header").count();
    let labels = md.match_indices("role=label").count();
    let values = md.match_indices("role=value").count();
    let unknowns = md.match_indices("role=unknown").count();

    assert!(headers > 0, "expected at least one header, got {headers}");
    assert!(values > headers, "values ({values}) should dominate headers ({headers})");
    assert!(labels > 0, "expected at least one label; classifier wiring regressed?");
    assert_eq!(
        unknowns, 0,
        "unknowns should not appear when classifier is wired: {unknowns}"
    );
}

#[test]
fn trl_llm_output_is_deterministic() {
    let fixture = repo_path(
        "test/260420-1. 연구개발계획서(서식)[TRL점프업 1단계]_대진대_수정_1430_fin.hwp",
    );
    let Ok(bytes) = std::fs::read(&fixture) else {
        eprintln!("skipping: TRL fixture not present");
        return;
    };
    let doc = HwpReader.read(&bytes).expect("read");
    let opts = markdown::MdOptions {
        llm: Some(markdown::LlmOptions::default()),
        ..markdown::MdOptions::default()
    };
    let md1 = markdown::to_markdown_with(&doc, &opts);
    let md2 = markdown::to_markdown_with(&doc, &opts);
    assert_eq!(md1, md2, "LLM output differed across runs");
}
