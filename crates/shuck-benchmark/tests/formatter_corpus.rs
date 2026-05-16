use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use shuck_benchmark::TEST_FILES;
use shuck_formatter::{
    FormattedSource, ShellDialect, ShellFormatOptions, format_file_ast, format_source,
    source_is_formatted,
};
use shuck_parser::parser::Parser;
use similar::TextDiff;

const BENCHMARK_ORACLE_FILE_COUNT: usize = 6;
const MAX_ORACLE_DIFF_LINES: usize = 200;
const KNOWN_BENCHMARK_DIVERGENCES: &[&str] = &[
    "homebrew-install",
    "ruby-build",
    "pyenv-python-build",
    "nvm",
    "bashtop",
];

#[test]
fn formatter_source_and_ast_paths_match_benchmark_corpus() {
    let options = ShellFormatOptions::default();

    for file in TEST_FILES.iter() {
        let filename = std::format!("{}.bash", file.name);
        let path = Path::new(&filename);
        let parsed = Parser::with_dialect(
            file.source,
            options.resolve(file.source, Some(path)).dialect(),
        )
        .parse()
        .unwrap();
        let from_source = format_source(file.source, Some(path), &options).unwrap();
        let from_ast = format_file_ast(file.source, parsed.file, Some(path), &options).unwrap();
        assert_eq!(from_source, from_ast);
        assert_eq!(
            source_is_formatted(file.source, Some(path), &options).unwrap(),
            matches!(from_source, FormattedSource::Unchanged)
        );
    }
}

#[test]
#[ignore = "requires SHUCK_RUN_SHFMT_ORACLE=1 and shfmt on PATH (for example via `nix develop`)"]
fn formatter_benchmark_corpus_matches_shfmt_baseline() {
    if std::env::var_os("SHUCK_RUN_SHFMT_ORACLE").is_none() {
        eprintln!("set SHUCK_RUN_SHFMT_ORACLE=1 to run the shfmt oracle");
        return;
    }

    probe_shfmt().expect("shfmt not found on PATH; run under `nix develop`");
    assert_eq!(
        TEST_FILES.len(),
        BENCHMARK_ORACLE_FILE_COUNT,
        "benchmark-backed oracle expects the benchmark corpus to stay at six scripts"
    );

    let options = ShellFormatOptions::default().with_dialect(ShellDialect::Bash);
    let mut unexpected_mismatches = Vec::new();
    let mut fixed_known_divergences = Vec::new();
    for file in TEST_FILES.iter() {
        let filename = format!("{}.bash", file.name);
        let shuck = run_shuck_formatter(file.source, &filename, &options);
        let shfmt = run_shfmt(file.source, &filename, &["-ln=bash"]);
        let mismatch = render_oracle_mismatch(file.name, &filename, &shfmt, &shuck);
        if KNOWN_BENCHMARK_DIVERGENCES.contains(&file.name) {
            if mismatch.is_none() {
                fixed_known_divergences.push(file.name);
            }
        } else if let Some(mismatch) = mismatch {
            unexpected_mismatches.push(mismatch);
        }
    }

    assert!(
        fixed_known_divergences.is_empty(),
        "benchmark corpus known divergences now match shfmt; remove from baseline: {}",
        fixed_known_divergences.join(", ")
    );
    assert!(
        unexpected_mismatches.is_empty(),
        "benchmark corpus introduced new shfmt divergence:\n\n{}",
        unexpected_mismatches.join("\n\n")
    );
}

fn probe_shfmt() -> Option<()> {
    let version = Command::new("shfmt").arg("--version").output().ok()?;
    if !version.status.success() {
        return None;
    }
    Some(())
}

fn run_shuck_formatter(source: &str, filename: &str, options: &ShellFormatOptions) -> String {
    match format_source(source, Some(Path::new(filename)), options).unwrap() {
        FormattedSource::Unchanged => source.to_string(),
        FormattedSource::Formatted(formatted) => formatted,
    }
}

fn run_shfmt(source: &str, filename: &str, flags: &[&str]) -> String {
    let mut command = Command::new("shfmt");
    command.args(flags);
    command.arg(format!("--filename={filename}"));
    command.stdin(Stdio::piped()).stdout(Stdio::piped());

    let mut child = command.spawn().expect("spawn shfmt");
    child
        .stdin
        .take()
        .expect("shfmt stdin")
        .write_all(source.as_bytes())
        .expect("write source to shfmt");
    let output = child.wait_with_output().expect("wait for shfmt");
    assert!(
        output.status.success(),
        "shfmt exited with {}",
        output.status
    );
    String::from_utf8(output.stdout).expect("utf8 shfmt output")
}

fn render_oracle_mismatch(name: &str, filename: &str, shfmt: &str, shuck: &str) -> Option<String> {
    if shfmt == shuck {
        return None;
    }

    let diff = TextDiff::from_lines(shfmt, shuck)
        .unified_diff()
        .header(&format!("shfmt/{filename}"), &format!("shuck/{filename}"))
        .context_radius(3)
        .to_string();
    let clipped = diff
        .lines()
        .take(MAX_ORACLE_DIFF_LINES)
        .collect::<Vec<_>>()
        .join("\n");

    Some(format!("case `{name}` failed:\n{clipped}"))
}
