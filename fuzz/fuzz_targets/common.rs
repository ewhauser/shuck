#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::Path;

use libfuzzer_sys::Corpus;
use shuck_ast::Span;
use shuck_formatter::{
    FormattedSource, ShellDialect as FormatDialect, ShellFormatOptions, format_file_ast,
    format_source, source_is_formatted,
};
use shuck_indexer::Indexer;
use shuck_linter::{Diagnostic, LinterSettings, lint_file_at_path_with_parse_result};
use shuck_parser::{ShellDialect as ParseDialect, parser::{ParseResult, Parser}};

const MAX_FUZZ_INPUT_BYTES: usize = 16 * 1024;
const MAX_FUZZ_NESTING: i32 = 64;

pub(crate) const PARSER_DIALECTS: [ParseDialect; 4] = [
    ParseDialect::Bash,
    ParseDialect::Posix,
    ParseDialect::Mksh,
    ParseDialect::Zsh,
];

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
    pub(crate) const fn new(
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

pub(crate) fn filtered_input(data: &[u8]) -> Result<&str, Corpus> {
    let input = std::str::from_utf8(data).map_err(|_| Corpus::Reject)?;
    if input.len() > MAX_FUZZ_INPUT_BYTES
        || max_nesting(input) > MAX_FUZZ_NESTING
        || contains_disallowed_controls(input)
    {
        return Err(Corpus::Reject);
    }
    Ok(input)
}

pub(crate) fn assert_span_valid(span: Span, source: &str) {
    assert!(
        span.start.offset <= span.end.offset,
        "invalid span ordering: {:?}",
        span
    );
    assert!(
        span.end.offset <= source.len(),
        "span end is out of bounds: {:?}",
        span
    );
    assert!(
        source.is_char_boundary(span.start.offset),
        "span start is not a char boundary: {:?}",
        span
    );
    assert!(
        source.is_char_boundary(span.end.offset),
        "span end is not a char boundary: {:?}",
        span
    );
}

pub(crate) fn recovered_parse_and_index(
    source: &str,
    dialect: ParseDialect,
) -> (ParseResult, Indexer) {
    let parse_result = Parser::with_dialect(source, dialect).parse();
    let indexer = Indexer::new(source, &parse_result);
    (parse_result, indexer)
}

pub(crate) fn format_result_to_string(result: FormattedSource, source: &str) -> String {
    match result {
        FormattedSource::Unchanged => source.to_string(),
        FormattedSource::Formatted(formatted) => formatted,
    }
}

pub(crate) fn lint_source_with_recovery(
    source: &str,
    path: Option<&Path>,
    dialect: ParseDialect,
) -> Vec<Diagnostic> {
    let parse_result = Parser::with_dialect(source, dialect).parse();
    let indexer = Indexer::new(source, &parse_result);
    let settings = match path {
        Some(path) => LinterSettings::default().with_analyzed_paths([path.to_path_buf()]),
        None => LinterSettings::default(),
    };
    lint_file_at_path_with_parse_result(
        &parse_result,
        source,
        &indexer,
        &settings,
        None,
        path,
    )
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
    lint_file_at_path_with_parse_result(
        &parse_result,
        source,
        &indexer,
        &settings,
        None,
        Some(path),
    )
}

pub(crate) fn compare_formatting_invariants(source: &str, case: FormatCase) {
    let path = Some(case.path());
    let options = case.format_options();

    let from_source = match format_source(source, path, &options) {
        Ok(result) => result,
        Err(shuck_formatter::FormatError::Parse { .. }) => return,
        Err(shuck_formatter::FormatError::Internal(message)) => {
            panic!(
                "internal formatter error for {}: {message}",
                case.path().display()
            )
        }
    };

    let parsed = Parser::with_dialect(source, case.parse_dialect())
        .parse();
    let parsed = if parsed.is_err() {
        panic!(
            "formatter accepted source but strict parsing failed for {}: {}",
            case.path().display(),
            parsed.strict_error()
        )
    } else {
        parsed
    };
    let from_ast = format_file_ast(source, parsed.file, path, &options).unwrap_or_else(|err| {
        panic!(
            "format_file_ast failed for {}: {err}",
            case.path().display()
        )
    });

    assert_eq!(
        from_source,
        from_ast,
        "format_source and format_file_ast diverged for {}",
        case.path().display()
    );

    let formatted_matches = source_is_formatted(source, path, &options).unwrap_or_else(|err| {
        panic!(
            "source_is_formatted failed for {}: {err}",
            case.path().display()
        )
    });
    assert_eq!(
        formatted_matches,
        matches!(from_source, FormattedSource::Unchanged),
        "source_is_formatted disagreed with formatter output for {}",
        case.path().display()
    );

    let once = format_result_to_string(from_source, source);
    let twice = format_source(&once, path, &options).unwrap_or_else(|err| {
        panic!(
            "second format pass failed for {}: {err}",
            case.path().display()
        )
    });
    let twice = format_result_to_string(twice, &once);

    assert_eq!(
        once,
        twice,
        "formatter was not idempotent for {}",
        case.path().display()
    );
    assert!(
        source_is_formatted(&once, path, &options).unwrap_or_else(|err| {
            panic!(
                "source_is_formatted rejected formatter output for {}: {err}",
                case.path().display()
            )
        }),
        "formatter output should be recognized as formatted for {}",
        case.path().display()
    );
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

fn contains_disallowed_controls(input: &str) -> bool {
    input
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
}

fn max_nesting(input: &str) -> i32 {
    input
        .bytes()
        .map(|byte| match byte {
            b'(' | b'{' | b'[' => 1,
            b')' | b'}' | b']' => -1,
            _ => 0,
        })
        .scan(0i32, |depth, delta| {
            *depth += delta;
            Some(*depth)
        })
        .max()
        .unwrap_or(0)
}
