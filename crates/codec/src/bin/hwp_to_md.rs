//! HWP → Markdown converter.
//!
//! ```sh
//! hwp-to-md [OPTIONS] <input.hwp> [output.md | -]
//! ```
//!
//! Options:
//!   --no-assets            Skip binary asset dump; no `![](…)` links emitted.
//!   --assets-dir=<path>    Write `BinaryEntry` payloads to <path> instead
//!                          of the default `<output-stem>.assets/`. Path may
//!                          be absolute or relative to CWD.
//!   -h, --help             Print this help and exit.
//!
//! Output targets:
//!   - `<output.md>` as a positional arg.
//!   - `-`  → stream Markdown to stdout. Assets default to **disabled** in
//!           this mode (there is no output directory to anchor them);
//!           pass `--assets-dir=<path>` to override.
//!   - omitted → `<input-stem>.md` next to the input.

use hwp_transpiler_codec::export::{assets, markdown};
use hwp_transpiler_codec::hwp::HwpReader;
use hwp_transpiler_core::ir::{IrDocument, Reader};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
usage: hwp-to-md [OPTIONS] <input.hwp> [output.md | -]

Options:
  --no-assets          Skip binary asset dump; no ![](…) links emitted.
  --assets-dir=<path>  Write assets to <path> (default: <out>.assets/).
  -h, --help           Print this help and exit.
";

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&raw) {
        Ok(Some(a)) => a,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("hwp-to-md: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    let bytes = match std::fs::read(&args.input) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("read {}: {e}", args.input.display());
            return ExitCode::from(2);
        }
    };

    let doc = match HwpReader.read(&bytes) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("parse {}: {e}", args.input.display());
            return ExitCode::from(1);
        }
    };

    let plan = resolve_output_plan(&args);

    let md_opts = markdown::MdOptions {
        assets_path: plan.assets_url_prefix.clone(),
    };
    let md = markdown::to_markdown_with(&doc, &md_opts);

    if let Some(dir) = &plan.assets_dir {
        match assets::dump_assets(&doc, dir) {
            Ok(written) => {
                if !written.is_empty() {
                    eprintln!(
                        "wrote {} asset{} → {}",
                        written.len(),
                        if written.len() == 1 { "" } else { "s" },
                        dir.display()
                    );
                }
            }
            Err(e) => {
                eprintln!("dump assets to {}: {e}", dir.display());
                return ExitCode::from(1);
            }
        }
    } else if !doc.bin_data.is_empty() && args.assets == AssetsMode::Disabled {
        eprintln!(
            "note: --no-assets set; {} embedded binar{} skipped",
            doc.bin_data.len(),
            if doc.bin_data.len() == 1 { "y" } else { "ies" }
        );
    }

    match &plan.output {
        OutputTarget::Stdout => {
            if let Err(e) = std::io::stdout().lock().write_all(md.as_bytes()) {
                eprintln!("write stdout: {e}");
                return ExitCode::from(1);
            }
        }
        OutputTarget::File(path) => {
            if let Err(e) = std::fs::write(path, md.as_bytes()) {
                eprintln!("write {}: {e}", path.display());
                return ExitCode::from(1);
            }
            eprintln!("wrote {} bytes → {}", md.len(), path.display());
        }
    }

    ExitCode::SUCCESS
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AssetsMode {
    Auto,
    Explicit(PathBuf),
    Disabled,
}

#[derive(Debug)]
struct CliArgs {
    input: PathBuf,
    output: OutputSpec,
    assets: AssetsMode,
}

#[derive(Debug)]
enum OutputSpec {
    Default,
    Stdout,
    File(PathBuf),
}

#[derive(Debug)]
enum OutputTarget {
    Stdout,
    File(PathBuf),
}

#[derive(Debug)]
struct OutputPlan {
    output: OutputTarget,
    assets_dir: Option<PathBuf>,
    /// Relative (when under the MD output's parent) or absolute URL used
    /// as the `MdOptions.assets_path` prefix so `![](prefix/BIN….ext)`
    /// resolves from wherever the Markdown lives.
    assets_url_prefix: Option<String>,
}

/// Parse argv. Returns `Ok(None)` on `-h/--help`, `Ok(Some(args))` on a
/// valid run, `Err` on a user error (printed with usage).
fn parse_args(raw: &[String]) -> Result<Option<CliArgs>, String> {
    let mut positionals: Vec<&str> = Vec::new();
    let mut assets = AssetsMode::Auto;

    let mut i = 0;
    while i < raw.len() {
        let a = raw[i].as_str();
        match a {
            "-h" | "--help" => return Ok(None),
            "--no-assets" => {
                if matches!(assets, AssetsMode::Explicit(_)) {
                    return Err(
                        "--no-assets and --assets-dir are mutually exclusive".into(),
                    );
                }
                assets = AssetsMode::Disabled;
            }
            s if s.starts_with("--assets-dir=") => {
                let val = &s["--assets-dir=".len()..];
                if val.is_empty() {
                    return Err("--assets-dir= requires a path".into());
                }
                if assets == AssetsMode::Disabled {
                    return Err(
                        "--no-assets and --assets-dir are mutually exclusive".into(),
                    );
                }
                assets = AssetsMode::Explicit(PathBuf::from(val));
            }
            "--assets-dir" => {
                let Some(val) = raw.get(i + 1) else {
                    return Err("--assets-dir requires a path".into());
                };
                if assets == AssetsMode::Disabled {
                    return Err(
                        "--no-assets and --assets-dir are mutually exclusive".into(),
                    );
                }
                assets = AssetsMode::Explicit(PathBuf::from(val));
                i += 1;
            }
            "--" => {
                for rest in &raw[i + 1..] {
                    positionals.push(rest);
                }
                break;
            }
            s if s.starts_with("--") => {
                return Err(format!("unknown option: {s}"));
            }
            s => positionals.push(s),
        }
        i += 1;
    }

    let input = match positionals.first() {
        Some(&s) => PathBuf::from(s),
        None => return Err("missing <input.hwp>".into()),
    };
    let output = match positionals.get(1) {
        None => OutputSpec::Default,
        Some(&"-") => OutputSpec::Stdout,
        Some(&s) => OutputSpec::File(PathBuf::from(s)),
    };
    if positionals.len() > 2 {
        return Err(format!("unexpected extra argument: {}", positionals[2]));
    }

    Ok(Some(CliArgs {
        input,
        output,
        assets,
    }))
}

/// Turn parsed args into an `OutputPlan`: concrete output target + where
/// (if anywhere) to dump assets and what URL prefix to embed in the MD.
fn resolve_output_plan(args: &CliArgs) -> OutputPlan {
    let output = match &args.output {
        OutputSpec::Default => OutputTarget::File(default_output_path(&args.input)),
        OutputSpec::Stdout => OutputTarget::Stdout,
        OutputSpec::File(p) => OutputTarget::File(p.clone()),
    };

    let assets_dir = match (&args.assets, &output) {
        (AssetsMode::Disabled, _) => None,
        (AssetsMode::Explicit(p), _) => Some(p.clone()),
        (AssetsMode::Auto, OutputTarget::File(md_path)) => Some(default_assets_dir(md_path)),
        // Auto + stdout: no natural anchor for a sidecar directory, so
        // stay safe and skip. Users who want assets from stdout mode pass
        // `--assets-dir=` explicitly.
        (AssetsMode::Auto, OutputTarget::Stdout) => None,
    };

    let assets_url_prefix = match (&assets_dir, &output) {
        (Some(dir), OutputTarget::File(md)) => Some(assets_prefix_for(dir, md)),
        (Some(dir), OutputTarget::Stdout) => Some(dir.to_string_lossy().into_owned()),
        (None, _) => None,
    };

    OutputPlan {
        output,
        assets_dir,
        assets_url_prefix,
    }
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

/// `<dir>/<stem>.md` → `<dir>/<stem>.assets`. Falls back to
/// `<dir>/assets` if the MD path has no stem (shouldn't happen from
/// `default_output_path`, but guards against `--output` overrides like
/// `hwp-to-md in.hwp /dev/stdout.md`).
fn default_assets_dir(md_path: &Path) -> PathBuf {
    let parent = md_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = md_path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("assets");
    parent.join(format!("{stem}.assets"))
}

/// Choose the URL prefix embedded in `![](prefix/BIN….ext)`. When the
/// assets dir lives under the MD output's parent, use the relative form
/// — that's what a Markdown renderer will resolve from the MD file's
/// location. Otherwise fall back to the path as given (absolute works in
/// most editors; a caller passing a cross-volume path gets what they
/// asked for).
fn assets_prefix_for(assets_dir: &Path, md_path: &Path) -> String {
    let md_parent = md_path.parent().unwrap_or_else(|| Path::new("."));
    if let Ok(rel) = assets_dir.strip_prefix(md_parent) {
        let rel_str = rel.to_string_lossy();
        if !rel_str.is_empty() {
            return rel_str.into_owned();
        }
    }
    assets_dir.to_string_lossy().into_owned()
}

#[allow(dead_code)]
fn _silence_unused(_doc: &IrDocument) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> CliArgs {
        let owned: Vec<String> = v.iter().map(|s| s.to_string()).collect();
        parse_args(&owned).expect("parse").expect("not help")
    }

    #[test]
    fn positional_only_defaults_to_auto_assets() {
        let a = args(&["in.hwp"]);
        assert_eq!(a.assets, AssetsMode::Auto);
        assert!(matches!(a.output, OutputSpec::Default));
        assert_eq!(a.input, PathBuf::from("in.hwp"));
    }

    #[test]
    fn stdout_target_recognised() {
        let a = args(&["in.hwp", "-"]);
        assert!(matches!(a.output, OutputSpec::Stdout));
    }

    #[test]
    fn no_assets_disables_dump() {
        let a = args(&["--no-assets", "in.hwp"]);
        assert_eq!(a.assets, AssetsMode::Disabled);
    }

    #[test]
    fn assets_dir_equals_form() {
        let a = args(&["--assets-dir=out/imgs", "in.hwp"]);
        match &a.assets {
            AssetsMode::Explicit(p) => assert_eq!(p, &PathBuf::from("out/imgs")),
            other => panic!("expected Explicit, got {other:?}"),
        }
    }

    #[test]
    fn assets_dir_space_form() {
        let a = args(&["--assets-dir", "out/imgs", "in.hwp"]);
        match &a.assets {
            AssetsMode::Explicit(p) => assert_eq!(p, &PathBuf::from("out/imgs")),
            other => panic!("expected Explicit, got {other:?}"),
        }
    }

    #[test]
    fn no_assets_and_explicit_conflict() {
        let e = parse_args(
            &["--no-assets", "--assets-dir=x", "in.hwp"]
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>(),
        )
        .expect_err("should conflict");
        assert!(e.contains("mutually exclusive"), "got: {e}");
    }

    #[test]
    fn help_short_circuits() {
        let owned: Vec<String> = vec!["--help".into()];
        assert!(parse_args(&owned).unwrap().is_none());
        let owned: Vec<String> = vec!["-h".into()];
        assert!(parse_args(&owned).unwrap().is_none());
    }

    #[test]
    fn missing_input_errors() {
        let e = parse_args(&[]).expect_err("no input");
        assert!(e.contains("missing <input.hwp>"), "got: {e}");
    }

    #[test]
    fn unknown_flag_errors() {
        let owned = vec!["--whatever".to_string(), "in.hwp".into()];
        let e = parse_args(&owned).expect_err("unknown");
        assert!(e.contains("unknown option"), "got: {e}");
    }

    #[test]
    fn default_output_path_rewrites_hwp_extension() {
        assert_eq!(
            default_output_path(Path::new("a/b/c.hwp")),
            PathBuf::from("a/b/c.md")
        );
        assert_eq!(
            default_output_path(Path::new("a/b/c.HWPX")),
            PathBuf::from("a/b/c.md")
        );
        assert_eq!(
            default_output_path(Path::new("noext")),
            PathBuf::from("noext.md")
        );
    }

    #[test]
    fn default_assets_dir_sits_next_to_md() {
        assert_eq!(
            default_assets_dir(Path::new("a/b/c.md")),
            PathBuf::from("a/b/c.assets")
        );
        assert_eq!(
            default_assets_dir(Path::new("c.md")),
            PathBuf::from("c.assets")
        );
    }

    #[test]
    fn relative_prefix_when_under_md_parent() {
        assert_eq!(
            assets_prefix_for(Path::new("a/b/c.assets"), Path::new("a/b/c.md")),
            "c.assets"
        );
    }

    #[test]
    fn absolute_prefix_when_cross_tree() {
        let p = assets_prefix_for(Path::new("/tmp/outside"), Path::new("a/b/c.md"));
        assert_eq!(p, "/tmp/outside");
    }

    #[test]
    fn plan_stdout_with_auto_disables_assets() {
        let a = args(&["in.hwp", "-"]);
        let plan = resolve_output_plan(&a);
        assert!(plan.assets_dir.is_none());
        assert!(plan.assets_url_prefix.is_none());
        assert!(matches!(plan.output, OutputTarget::Stdout));
    }

    #[test]
    fn plan_stdout_with_explicit_keeps_assets() {
        let a = args(&["--assets-dir=out", "in.hwp", "-"]);
        let plan = resolve_output_plan(&a);
        assert_eq!(plan.assets_dir, Some(PathBuf::from("out")));
        assert_eq!(plan.assets_url_prefix.as_deref(), Some("out"));
    }

    #[test]
    fn plan_default_auto_produces_sibling_assets_dir() {
        let a = args(&["some/path/doc.hwp"]);
        let plan = resolve_output_plan(&a);
        assert!(
            matches!(&plan.output, OutputTarget::File(p) if p == Path::new("some/path/doc.md"))
        );
        assert_eq!(plan.assets_dir, Some(PathBuf::from("some/path/doc.assets")));
        assert_eq!(plan.assets_url_prefix.as_deref(), Some("doc.assets"));
    }

    #[test]
    fn plan_no_assets_yields_no_dir_and_no_prefix() {
        let a = args(&["--no-assets", "doc.hwp"]);
        let plan = resolve_output_plan(&a);
        assert!(plan.assets_dir.is_none());
        assert!(plan.assets_url_prefix.is_none());
    }
}
