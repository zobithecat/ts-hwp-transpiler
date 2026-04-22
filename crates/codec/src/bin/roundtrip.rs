//! Semi-automatic round-trip runner.
//!
//! Usage:
//!   hwp-roundtrip <fixture.hwp>
//!
//! Reads a fixture HWP file, runs it through our reader+writer, and reports
//! the first stream-level byte divergence with hex context. Exit code 0 on
//! match, 1 on any mismatch (friendly with `cargo watch`, `entr`, etc.).
//!
//! Paired with scripts/watch-roundtrip.sh to form the inner dev loop for
//! task #10 (DocInfo writer port).

use hwp_transpiler_codec::hwp::{HwpReader, HwpWriter};
use hwp_transpiler_core::ir::{Reader, Writer};
use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1).map(PathBuf::from) else {
        eprintln!("usage: hwp-roundtrip <fixture.hwp>");
        return ExitCode::from(2);
    };

    let fixture = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("read {}: {e}", path.display());
            return ExitCode::from(2);
        }
    };

    let ir = match HwpReader.read(&fixture) {
        Ok(ir) => ir,
        Err(e) => {
            eprintln!("[reader] {e}");
            return ExitCode::from(1);
        }
    };

    let out = match HwpWriter.write(&ir) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[writer] {e}");
            return ExitCode::from(1);
        }
    };

    let lhs = match stream_map(&fixture) {
        Ok(m) => m,
        Err(e) => { eprintln!("[fixture] parse: {e}"); return ExitCode::from(1); }
    };
    let rhs = match stream_map(&out) {
        Ok(m) => m,
        Err(e) => { eprintln!("[output] parse: {e}"); return ExitCode::from(1); }
    };

    print_summary(&path, &lhs, &rhs);

    let mut mismatches = 0usize;
    for name in lhs.keys() {
        if let Some(r) = rhs.get(name) {
            let l = &lhs[name];
            if l != r {
                mismatches += 1;
                report_first_divergence(name, l, r);
            }
        } else {
            mismatches += 1;
            eprintln!("missing stream in output: {name}");
        }
    }
    for name in rhs.keys() {
        if !lhs.contains_key(name) {
            mismatches += 1;
            eprintln!("extra stream in output: {name}");
        }
    }

    if mismatches == 0 {
        println!("\n\u{2713} round-trip OK ({} streams)", lhs.len());
        ExitCode::SUCCESS
    } else {
        eprintln!("\n\u{2717} {mismatches} stream(s) diverge");
        ExitCode::FAILURE
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

fn print_summary(
    path: &std::path::Path,
    lhs: &BTreeMap<String, Vec<u8>>,
    rhs: &BTreeMap<String, Vec<u8>>,
) {
    println!("fixture: {}", path.display());
    println!("  streams: fixture={}, output={}", lhs.len(), rhs.len());
    for (name, bytes) in lhs {
        let out_len = rhs.get(name).map(|v| v.len() as isize).unwrap_or(-1);
        let marker = if out_len == bytes.len() as isize { "=" } else { "!" };
        println!("  {marker} {name}: fixture={} output={}", bytes.len(), out_len);
    }
}

fn report_first_divergence(name: &str, a: &[u8], b: &[u8]) {
    let idx = a.iter().zip(b.iter()).position(|(x, y)| x != y)
        .or_else(|| if a.len() != b.len() { Some(a.len().min(b.len())) } else { None });
    let Some(i) = idx else { return };

    let start = i.saturating_sub(16);
    let end_a = (i + 16).min(a.len());
    let end_b = (i + 16).min(b.len());

    eprintln!("\n[{name}] first divergence at offset 0x{i:08X} ({i})");
    eprintln!("  fixture  : {}", hex(&a[start..end_a], i - start));
    eprintln!("  output   : {}", hex(&b[start..end_b], i - start));
    eprintln!("  len fix={}, out={}", a.len(), b.len());
}

fn hex(bytes: &[u8], caret: usize) -> String {
    let mut s = String::new();
    for (n, b) in bytes.iter().enumerate() {
        s.push_str(&format!("{}{b:02X} ", if n == caret { ">" } else { " " }));
    }
    s
}
