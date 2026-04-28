use std::collections::BTreeMap;
use std::path::Path;

use shuck_formatter::{FormattedSource, ShellDialect as FormatDialect, ShellFormatOptions};
use shuck_indexer::Indexer;
use shuck_linter::{Diagnostic, LinterSettings, lint_file};
use shuck_parser::{ShellDialect as ParseDialect, parser::Parser};

pub(crate) const FORMAT_CASES: [FormatCase; 4] = [
    FormatCase::new("fuzz.sh", ParseDialect::Posix, FormatDialect::Auto),
    FormatCase::new("fuzz.bash", ParseDialect::Bash, FormatDialect::Auto),
    FormatCase::new("fuzz.mksh", ParseDialect::Mksh, FormatDialect::Auto),
    FormatCase::new("fuzz.zsh", ParseDialect::Zsh, FormatDialect::Auto),
];

#[derive(Clone, Copy)]
pub(crate) struct FormatCase {
    path: &'static str,
    parse_dialect: ParseDialect,
    format_dialect: FormatDialect,
}

impl FormatCase {
    const fn new(
        path: &'static str,
        parse_dialect: ParseDialect,
        format_dialect: FormatDialect,
    ) -> Self {
        Self {
            path,
            parse_dialect,
            format_dialect,
        }
    }

    pub(crate) fn path(self) -> &'static Path {
        Path::new(self.path)
    }

    pub(crate) fn parse_dialect(self) -> ParseDialect {
        self.parse_dialect
    }

    pub(crate) fn format_options(self) -> ShellFormatOptions {
        ShellFormatOptions::default().with_dialect(self.format_dialect)
    }
}

pub(crate) fn format_result_to_string(result: FormattedSource, source: &str) -> String {
    match result {
        FormattedSource::Unchanged => source.to_string(),
        FormattedSource::Formatted(formatted) => formatted,
    }
}

pub(crate) fn lint_source_strict(
    source: &str,
    path: &Path,
    dialect: ParseDialect,
) -> Vec<Diagnostic> {
    let parse_result = Parser::with_dialect(source, dialect).parse();
    if parse_result.is_err() {
        panic!(
            "strict parse failed for {}: {}",
            path.display(),
            parse_result.strict_error()
        );
    }
    let indexer = Indexer::new(source, &parse_result);
    let settings = LinterSettings::default().with_analyzed_paths([path.to_path_buf()]);
    lint_file(&parse_result, source, &indexer, &settings, None, Some(path))
}

pub(crate) fn compare_lint_counts(original: &[Diagnostic], formatted: &[Diagnostic], path: &Path) {
    let original_counts = diagnostic_counts(original);
    let formatted_counts = diagnostic_counts(formatted);

    for (code, formatted_count) in formatted_counts {
        let original_count = original_counts.get(code).copied().unwrap_or(0);
        assert!(
            formatted_count <= original_count,
            "formatter introduced additional {code} diagnostics for {}",
            path.display()
        );
    }
}

fn diagnostic_counts<'a>(diagnostics: &'a [Diagnostic]) -> BTreeMap<&'a str, usize> {
    let mut counts = BTreeMap::new();
    for diagnostic in diagnostics {
        *counts.entry(diagnostic.code()).or_default() += 1;
    }
    counts
}
