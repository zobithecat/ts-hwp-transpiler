//! Demo CLI: `.hwp` → Markdown on stdout.
//!
//! ```sh
//! cargo run -p hwp-transpiler-codec --bin hwp-to-md -- test/fixture.hwp > out.md
//! ```

use hwp_transpiler_codec::export::markdown;
use hwp_transpiler_codec::hwp::HwpReader;
use hwp_transpiler_core::ir::Reader;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        eprintln!("usage: hwp-to-md <input.hwp>");
        return ExitCode::from(2);
    };

    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("read {path}: {e}");
            return ExitCode::from(2);
        }
    };

    let doc = match HwpReader.read(&bytes) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("parse {path}: {e}");
            return ExitCode::from(1);
        }
    };

    print!("{}", markdown::to_markdown(&doc));
    ExitCode::SUCCESS
}
