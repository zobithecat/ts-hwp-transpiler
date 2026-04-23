//! Integration test: run `semantics::infer_table_domain` against every
//! table in a real `연구개발계획서` HWP fixture and assert a reasonable
//! classification distribution.
//!
//! The fixture is the same TRL proposal form used by `round_trip.rs`
//! (gitignored — skip with an eprintln if absent). Exact counts would
//! be brittle as the fixture evolves; instead we assert *shape*
//! invariants that prove the classifier is meaningfully engaging with
//! real content:
//!   - ≥ 10 tables were seen.
//!   - ≥ 30 % classified as non-Unknown.
//!   - ≥ 3 distinct core proposal-form domains (Budget, Schedule,
//!     Personnel, Performance, Institution) are represented.
//!
//! The histogram is printed via `eprintln!` so readers sanity-check
//! drift when the fixture or classifier changes.
//!
//! Observed at introduction (53 tables, 23 classified):
//!   institution_info=3, budget=5, schedule=2, performance_metrics=5,
//!   personnel=8, unknown=30.

use hwp_transpiler_codec::hwp::HwpReader;
use hwp_transpiler_core::ir::{Control, ControlKind, IrDocument, Reader};
use hwp_transpiler_core::semantics::{TableDomain, infer_table_domain};
use std::path::PathBuf;

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// Walk the whole document and classify every table, passing each
/// table's owning paragraph text so `infer_table_domain` sees the
/// "section heading" signal that real forms rely on. Nested tables
/// are classified independently from their wrappers so the histogram
/// reflects every distinct grid the fixture contains.
fn classify_all_tables(doc: &IrDocument) -> Vec<TableDomain> {
    let mut out: Vec<TableDomain> = Vec::new();
    for section in &doc.sections {
        for para in &section.paragraphs {
            walk_controls(&para.controls, &para.text, &mut out);
        }
    }
    out
}

fn walk_controls(controls: &[Control], owner_text: &str, out: &mut Vec<TableDomain>) {
    for c in controls {
        if let ControlKind::Table(t) = &c.kind {
            out.push(infer_table_domain(t, owner_text));
            for cell in &t.cells {
                for p in &cell.paragraphs {
                    walk_controls(&p.controls, &p.text, out);
                }
            }
        }
    }
}

#[test]
fn trl_proposal_tables_classify_sensibly() {
    let Ok(fixture) = std::fs::read(repo_path(
        "test/260420-1. 연구개발계획서(서식)[TRL점프업 1단계]_대진대_수정_1430_fin.hwp",
    )) else {
        eprintln!("skipping: TRL fixture not present");
        return;
    };
    let doc = HwpReader.read(&fixture).expect("read HWP");
    let classified = classify_all_tables(&doc);

    assert!(
        classified.len() >= 10,
        "expected ≥ 10 tables in proposal form, got {}",
        classified.len()
    );

    let mut histogram: [(TableDomain, usize); 6] = [
        (TableDomain::Institution, 0),
        (TableDomain::Budget, 0),
        (TableDomain::Schedule, 0),
        (TableDomain::Performance, 0),
        (TableDomain::Personnel, 0),
        (TableDomain::Unknown, 0),
    ];
    for domain in &classified {
        for slot in histogram.iter_mut() {
            if slot.0 == *domain {
                slot.1 += 1;
                break;
            }
        }
    }

    eprintln!("table domain histogram ({} tables total):", classified.len());
    for (d, n) in &histogram {
        eprintln!("  {:<18} {}", d.as_str(), n);
    }

    let classified_count: usize = histogram
        .iter()
        .filter(|(d, _)| *d != TableDomain::Unknown)
        .map(|(_, n)| *n)
        .sum();
    let ratio_x100 = classified_count * 100 / classified.len();
    assert!(
        ratio_x100 >= 30,
        "classified fraction too low: {}/{} = {}%",
        classified_count,
        classified.len(),
        ratio_x100
    );

    let hit_domains = histogram
        .iter()
        .filter(|(d, n)| *d != TableDomain::Unknown && *n > 0)
        .count();
    assert!(
        hit_domains >= 3,
        "expected ≥ 3 distinct classified domains, got {hit_domains}"
    );
}
