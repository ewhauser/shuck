use std::path::Path;

use shuck_formatter::{
    FormattedSource, ShellDialect as FormatDialect, ShellFormatOptions, format_file_ast,
    format_source, source_is_formatted,
};
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

    fn path(self) -> &'static Path {
        Path::new(self.path)
    }

    fn parse_dialect(self) -> ParseDialect {
        self.parse_dialect
    }

    fn format_options(self) -> ShellFormatOptions {
        ShellFormatOptions::default().with_dialect(self.format_dialect)
    }
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

    let parsed = Parser::with_dialect(source, case.parse_dialect()).parse();
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

fn format_result_to_string(result: FormattedSource, source: &str) -> String {
    match result {
        FormattedSource::Unchanged => source.to_string(),
        FormattedSource::Formatted(formatted) => formatted,
    }
}
