//! HWP → Markdown converter.
//!
//! ```sh
//! hwp-to-md <input.hwp> [output.md]
//! ```
//!
//! If `[output.md]` is omitted, writes to `<input>.md` (same dir, `.hwp`
//! replaced with `.md`). Pass `-` to stream to stdout instead.

use hwp_transpiler_codec::export::markdown;
use hwp_transpiler_codec::hwp::HwpReader;
use hwp_transpiler_core::ir::Reader;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let Some(input) = args.get(1) else {
        eprintln!("usage: hwp-to-md <input.hwp> [output.md | -]");
        return ExitCode::from(2);
    };

    let input_path = Path::new(input);
    let bytes = match std::fs::read(input_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("read {}: {e}", input_path.display());
            return ExitCode::from(2);
        }
    };

    let doc = match HwpReader.read(&bytes) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("parse {}: {e}", input_path.display());
            return ExitCode::from(1);
        }
    };

    let md = markdown::to_markdown(&doc);

    match args.get(2).map(String::as_str) {
        Some("-") => {
            if let Err(e) = std::io::stdout().lock().write_all(md.as_bytes()) {
                eprintln!("write stdout: {e}");
                return ExitCode::from(1);
            }
        }
        explicit => {
            let out_path = match explicit {
                Some(s) => PathBuf::from(s),
                None => default_output_path(input_path),
            };
            if let Err(e) = std::fs::write(&out_path, md.as_bytes()) {
                eprintln!("write {}: {e}", out_path.display());
                return ExitCode::from(1);
            }
            eprintln!(
                "wrote {} bytes → {}",
                md.len(),
                out_path.display()
            );
        }
    }

    ExitCode::SUCCESS
}

/// `path/to/file.hwp` → `path/to/file.md`. Any other extension (or none) →
/// appends `.md`.
fn default_output_path(input: &Path) -> PathBuf {
    let mut out = input.to_path_buf();
    let is_hwp = out
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("hwp") || e.eq_ignore_ascii_case("hwpx"))
        .unwrap_or(false);
    if is_hwp {
        out.set_extension("md");
    } else {
        let fname = format!(
            "{}.md",
            out.file_name().and_then(|f| f.to_str()).unwrap_or("out")
        );
        out.set_file_name(fname);
    }
    out
}
