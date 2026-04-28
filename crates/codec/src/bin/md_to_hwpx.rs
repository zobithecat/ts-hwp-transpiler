//! Markdown → HWPX converter.
//!
//! ```sh
//! md-to-hwpx <input.md> [output.hwpx | -]
//! ```
//!
//! Output targets:
//!   - `<output.hwpx>` as a positional arg.
//!   - `-`  → stream HWPX bytes to stdout (binary; pipe to a file).
//!   - omitted → `<input-stem>.hwpx` next to the input.
//!
//! Limitations of the current importer (see `import::markdown`):
//!   - Headings + paragraphs + GFM pipe tables are decoded.
//!   - Lists / images / inline emphasis as typed runs / hint
//!     annotations are not yet — they fall through as plain text.
//!
//! The bundled HWPX skeleton (`hwpx::skeleton`) supplies
//! `META-INF/container.xml`, `Contents/content.hpf`, and a minimal
//! `Contents/header.xml` so the output opens in HWPX viewers, not
//! just in our own reader.

use hwp_transpiler_codec::hwpx::skeleton::bundle_default_skeleton;
use hwp_transpiler_codec::hwpx::HwpxWriter;
use hwp_transpiler_codec::import::markdown::from_markdown;
use hwp_transpiler_core::ir::Writer;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
usage: md-to-hwpx <input.md> [output.hwpx | -]

Reads UTF-8 Markdown, writes a minimum-viable HWPX. Output defaults
to <input-stem>.hwpx next to the input. Pass `-` to stream the HWPX
bytes (binary) to stdout.

Asset auto-pairing: if a sibling `<stem>.assets.md` lives next to
the input (i.e. it was produced by `hwp-to-md --split-assets`), it's
read and concatenated automatically so embedded pictures round-trip
into the HWPX without extra flags.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if args.is_empty() {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }

    let input_path = PathBuf::from(&args[0]);
    let output_target = args.get(1).cloned();

    let main_src = match std::fs::read_to_string(&input_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read {}: {e}", input_path.display());
            return ExitCode::from(2);
        }
    };

    // Look for a sibling `<stem>.assets.md` companion (split-asset
    // mode emit) and append it to the main MD before parsing. The
    // LLM importer's `<!-- hwp-transpiler: assets -->` footer
    // detection treats the appended block exactly like an inline
    // footer — a single parser path covers both modes.
    let src = match load_companion_assets(&input_path) {
        Some(assets) => {
            eprintln!(
                "found {} bytes of companion assets",
                assets.len()
            );
            format!("{main_src}\n\n{assets}")
        }
        None => main_src,
    };

    let mut doc = match from_markdown(&src) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("parse {}: {e}", input_path.display());
            return ExitCode::from(1);
        }
    };
    bundle_default_skeleton(&mut doc);

    let bytes = match HwpxWriter::default().write(&doc) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("write hwpx: {e}");
            return ExitCode::from(1);
        }
    };

    match output_target.as_deref() {
        Some("-") => {
            if let Err(e) = std::io::stdout().write_all(&bytes) {
                eprintln!("stdout write: {e}");
                return ExitCode::from(1);
            }
        }
        Some(path) => {
            if let Err(e) = std::fs::write(path, &bytes) {
                eprintln!("write {path}: {e}");
                return ExitCode::from(1);
            }
        }
        None => {
            let mut out = input_path.clone();
            out.set_extension("hwpx");
            if let Err(e) = std::fs::write(&out, &bytes) {
                eprintln!("write {}: {e}", out.display());
                return ExitCode::from(1);
            }
            eprintln!("wrote {}", out.display());
        }
    }

    ExitCode::SUCCESS
}

/// Resolve `<stem>.md` → `<stem>.assets.md` and read it back when
/// present. Silently returns `None` if the file is absent — the
/// importer falls back to assets-less behaviour. Quiet on read
/// errors too: a sibling that exists but won't decode shouldn't
/// kill the conversion of the main file.
fn load_companion_assets(main: &std::path::Path) -> Option<String> {
    let parent = main.parent().unwrap_or_else(|| std::path::Path::new(""));
    let stem = main.file_stem().and_then(|s| s.to_str())?;
    let companion = parent.join(format!("{stem}.assets.md"));
    std::fs::read_to_string(companion).ok()
}
