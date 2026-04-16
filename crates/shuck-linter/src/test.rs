use std::fmt::Write;
use std::fs;
use std::path::Path;

use shuck_indexer::Indexer;
use shuck_parser::ShellProfile;
use shuck_parser::parser::{ParseResult, Parser, ShellDialect as ParseShellDialect};

use crate::{Diagnostic, LinterSettings, lint_file_at_path_with_parse_result};

fn inferred_shell_profile(
    source: &str,
    settings: &LinterSettings,
    path: Option<&Path>,
) -> ShellProfile {
    let shell = if settings.shell == crate::ShellDialect::Unknown {
        crate::ShellDialect::infer(source, path)
    } else {
        settings.shell
    };
    let dialect = match shell {
        crate::ShellDialect::Zsh => ParseShellDialect::Zsh,
        crate::ShellDialect::Unknown
        | crate::ShellDialect::Sh
        | crate::ShellDialect::Dash
        | crate::ShellDialect::Ksh
        | crate::ShellDialect::Mksh
        | crate::ShellDialect::Bash => ParseShellDialect::Bash,
    };
    ShellProfile::native(dialect)
}

fn parse_for_lint(source: &str, settings: &LinterSettings, path: Option<&Path>) -> ParseResult {
    Parser::with_profile(source, inferred_shell_profile(source, settings, path)).parse()
}

/// Lint a source string directly (no file needed).
pub fn test_snippet(source: &str, settings: &LinterSettings) -> Vec<Diagnostic> {
    let parse_result = parse_for_lint(source, settings, None);
    let indexer = Indexer::new(source, &parse_result);
    lint_file_at_path_with_parse_result(&parse_result, source, &indexer, settings, None, None)
}

/// Lint a source string while preserving an explicit path for path-sensitive rules.
pub fn test_snippet_at_path(
    path: &Path,
    source: &str,
    settings: &LinterSettings,
) -> Vec<Diagnostic> {
    let parse_result = parse_for_lint(source, settings, Some(path));
    let indexer = Indexer::new(source, &parse_result);
    lint_file_at_path_with_parse_result(&parse_result, source, &indexer, settings, None, Some(path))
}

/// Lint a fixture file relative to `resources/test/fixtures/`.
///
/// Returns diagnostics and the source text (needed for snapshot formatting).
pub fn test_path(
    path: &Path,
    settings: &LinterSettings,
) -> anyhow::Result<(Vec<Diagnostic>, String)> {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/test/fixtures");
    let full_path = fixtures_dir.join(path);
    let source = fs::read_to_string(&full_path)?;
    let settings = if settings.analyzed_paths.is_some() {
        settings.clone()
    } else {
        let analyzed_paths = full_path
            .parent()
            .into_iter()
            .flat_map(|dir| fs::read_dir(dir).into_iter().flatten())
            .flatten()
            .filter_map(|entry| {
                entry
                    .file_type()
                    .ok()
                    .and_then(|kind| kind.is_file().then_some(entry.path()))
            })
            .collect::<Vec<_>>();
        settings.clone().with_analyzed_paths(analyzed_paths)
    };
    let diagnostics = test_snippet_at_path(&full_path, &source, &settings);
    Ok((diagnostics, source))
}

/// Format diagnostics for snapshot comparison.
///
/// Output format (one block per diagnostic):
/// ```text
/// C001 variable `foo` is assigned but never used
///  --> C001.sh:2:1
///   |
/// 2 | foo=1
///   |
/// ```
pub fn print_diagnostics(diagnostics: &[Diagnostic], source: &str) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let mut output = String::new();

    for (i, diagnostic) in diagnostics.iter().enumerate() {
        if i > 0 {
            output.push('\n');
        }

        let line = diagnostic.span.start.line;
        let col = diagnostic.span.start.column;
        let line_width = line.to_string().len();

        // Rule code + message
        writeln!(output, "{} {}", diagnostic.code(), diagnostic.message).unwrap();

        // Location
        writeln!(
            output,
            "{:width$}--> <source>:{line}:{col}",
            " ",
            width = line_width,
        )
        .unwrap();

        // Source context
        writeln!(output, "{:width$} |", " ", width = line_width).unwrap();

        if line > 0 && line <= lines.len() {
            let source_line = lines[line - 1];
            writeln!(output, "{line} | {source_line}").unwrap();
        }

        writeln!(output, "{:width$} |", " ", width = line_width).unwrap();
    }

    output
}

/// Assert diagnostics match a named snapshot.
///
/// # Examples
///
/// ```ignore
/// // Named snapshot (stored in snapshots/ directory)
/// assert_diagnostics!("C001_basic", diagnostics, source);
///
/// // Inline snapshot
/// assert_diagnostics!(diagnostics, source, @"expected output");
/// ```
#[macro_export]
macro_rules! assert_diagnostics {
    ($name:expr, $diagnostics:expr, $source:expr) => {{
        insta::with_settings!({ omit_expression => true }, {
            insta::assert_snapshot!($name, $crate::test::print_diagnostics(&$diagnostics, $source));
        });
    }};
    ($diagnostics:expr, $source:expr, @$snapshot:literal) => {{
        insta::with_settings!({ omit_expression => true }, {
            insta::assert_snapshot!($crate::test::print_diagnostics(&$diagnostics, $source), @$snapshot);
        });
    }};
    ($diagnostics:expr, $source:expr) => {{
        insta::with_settings!({ omit_expression => true }, {
            insta::assert_snapshot!($crate::test::print_diagnostics(&$diagnostics, $source));
        });
    }};
}
