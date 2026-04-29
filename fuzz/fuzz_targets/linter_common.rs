use std::path::Path;

use shuck_ast::Span;
use shuck_indexer::Indexer;
use shuck_linter::{Diagnostic, LinterSettings, ShellCheckCodeMap, lint_file};
use shuck_parser::{ShellDialect as ParseDialect, parser::Parser};

pub(crate) const LINT_CASES: [LintCase; 4] = [
    LintCase::new("fuzz.sh", ParseDialect::Posix),
    LintCase::new("fuzz.bash", ParseDialect::Bash),
    LintCase::new("fuzz.mksh", ParseDialect::Mksh),
    LintCase::new("fuzz.zsh", ParseDialect::Zsh),
];

#[derive(Clone, Copy)]
pub(crate) struct LintCase {
    path: &'static str,
    parse_dialect: ParseDialect,
}

impl LintCase {
    const fn new(path: &'static str, parse_dialect: ParseDialect) -> Self {
        Self {
            path,
            parse_dialect,
        }
    }

    pub(crate) fn path(self) -> &'static Path {
        Path::new(self.path)
    }

    pub(crate) fn parse_dialect(self) -> ParseDialect {
        self.parse_dialect
    }
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
    lint_file(
        &parse_result,
        source,
        &indexer,
        &settings,
        &ShellCheckCodeMap::default(),
        path,
    )
}
