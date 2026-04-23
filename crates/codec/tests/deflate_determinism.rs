//! Phase 4 probe: can `flate2` reproduce a real HWP's DEFLATE output
//! byte-for-byte? hwplib runs Java's `Deflater(DEFAULT_COMPRESSION)`
//! with no zlib header; we run `flate2::write::DeflateEncoder`, same
//! raw-DEFLATE framing. Same RFC 1951 standard — but different
//! encoder implementations are allowed to pick different codewords,
//! sliding-window decisions, and block boundaries. The only way to
//! find out is to try every exposed knob.
//!
//! Target streams with `FileHeader.compressed = 1`:
//!   - `test/fixture.hwp` (hwplib-produced small compressed fixture)
//!   - `test/260420-1. 연구개발계획서...hwp` (TRL, large real-world)
//!
//! Both are gitignored, so we skip gracefully when absent and print
//! what we could observe. `fixtures/blank.hwp` is skipped entirely —
//! its FileHeader has `compressed=0`, streams are raw (not a useful
//! target for this experiment).
//!
//! If any `flate2` configuration round-trips byte-equal, we adopt it
//! in `doc_info::compress` so unmutated re-encode recovers full byte
//! equality with the original stream (not just structural). If no
//! configuration works, we document that here so future contributors
//! don't repeat the experiment.

use cfb::CompoundFile;
use flate2::Compression;
use flate2::write::DeflateEncoder;
use hwp_transpiler_codec::hwp::streams::doc_info;
use std::io::{Cursor, Read, Write};
use std::path::PathBuf;

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// Extract the raw (compressed) `/DocInfo` stream bytes from the
/// given HWP file. Returns `None` when the fixture is absent or the
/// file's FileHeader reports uncompressed streams.
fn docinfo_compressed(path: &PathBuf) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    let mut comp = CompoundFile::open(Cursor::new(bytes)).ok()?;
    // FileHeader byte 36 is the first byte of `flags` (LE u32); bit 0
    // is the compressed flag. Peek at it directly instead of going
    // through the typed reader, so we stay independent of IR quirks.
    let mut fh = comp.open_stream("/FileHeader").ok()?;
    let mut header = Vec::with_capacity(256);
    fh.read_to_end(&mut header).ok()?;
    drop(fh);
    let flags = u32::from_le_bytes(header[36..40].try_into().ok()?);
    if flags & 0x01 == 0 {
        return None;
    }
    let mut s = comp.open_stream("/DocInfo").ok()?;
    let mut v = Vec::with_capacity(s.len() as usize);
    s.read_to_end(&mut v).ok()?;
    Some(v)
}

fn compress_with_level(raw: &[u8], level: u32) -> Vec<u8> {
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::new(level));
    enc.write_all(raw).expect("write");
    enc.finish().expect("finish")
}

fn sweep(label: &str, hwplib: &[u8]) -> bool {
    let raw = doc_info::decompress(hwplib).expect("decompress");
    eprintln!(
        "{label}: {} compressed bytes → {} raw bytes",
        hwplib.len(),
        raw.len()
    );

    let mut any_match = false;
    for level in 0..=9u32 {
        let ours = compress_with_level(&raw, level);
        let matches = ours == *hwplib;
        eprintln!(
            "  level {level}: {} bytes{}{}",
            ours.len(),
            if ours.len() == hwplib.len() {
                " (same length)"
            } else {
                ""
            },
            if matches { "  ✓ BYTE-EQUAL" } else { "" }
        );
        if matches {
            any_match = true;
        }
        // Also require that our output decompresses back to the
        // same raw bytes — the real correctness invariant.
        let back = doc_info::decompress(&ours)
            .unwrap_or_else(|e| panic!("{label} level {level}: re-decompress failed: {e}"));
        assert_eq!(
            back, raw,
            "{label} level {level}: DEFLATE round-trip corrupted payload"
        );
    }
    any_match
}

#[test]
fn deflate_byte_equality_experiment() {
    let mut any_fixture_matched = false;
    let mut ran_at_least_one = false;

    for rel in [
        "test/fixture.hwp",
        "test/260420-1. 연구개발계획서(서식)[TRL점프업 1단계]_대진대_수정_1430_fin.hwp",
    ] {
        let path = repo_path(rel);
        let Some(hwplib) = docinfo_compressed(&path) else {
            eprintln!("skipping: {rel} (absent or not compressed)");
            continue;
        };
        ran_at_least_one = true;
        if sweep(rel, &hwplib) {
            any_fixture_matched = true;
        }
    }

    if !ran_at_least_one {
        eprintln!(
            "no compressed fixture present — experiment inconclusive. \
             Drop a hwplib-produced compressed HWP under /test/ to run."
        );
        return;
    }

    if any_fixture_matched {
        eprintln!(
            "\nresult: at least one flate2 level produces byte-equal \
             output vs hwplib. Consider adopting that level in \
             doc_info::compress."
        );
    } else {
        eprintln!(
            "\nresult: no flate2 level produces byte-equal output vs \
             hwplib across the tested fixtures. Decompression round-\
             trip still holds for all levels — structural equality \
             remains the invariant, byte-equality is not achievable \
             via the flate2 knobs alone."
        );
    }
}
