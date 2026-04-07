mod command;
mod comments;
mod context;
mod generated;
mod options;
mod prelude;
mod redirect;
mod script;
mod shared_traits;
mod simplify;
mod word;

use std::path::Path;

use shuck_format::{FormatResult, format};
use shuck_parser::{Error as ParseError, parser::Parser};

use crate::shared_traits::AsFormat;

pub use crate::options::{ResolvedShellFormatOptions, ShellDialect, ShellFormatOptions};
pub use shuck_format::IndentStyle;

pub type ShellFormatter<'source, 'buf> =
    shuck_format::Formatter<context::ShellFormatContext<'source>>;

pub trait FormatNodeRule<N> {
    fn fmt(&self, node: &N, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormattedSource {
    Unchanged,
    Formatted(String),
}

impl FormattedSource {
    #[must_use]
    pub fn is_changed(&self) -> bool {
        matches!(self, Self::Formatted(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatError {
    Parse {
        message: String,
        line: usize,
        column: usize,
    },
    Internal(String),
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse {
                message,
                line,
                column,
            } => {
                if *line > 0 {
                    write!(f, "parse error at line {line}, column {column}: {message}")
                } else {
                    write!(f, "parse error: {message}")
                }
            }
            Self::Internal(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for FormatError {}

pub type Result<T> = std::result::Result<T, FormatError>;

pub fn format_source(
    source: &str,
    path: Option<&Path>,
    options: &ShellFormatOptions,
) -> Result<FormattedSource> {
    let resolved = options.resolve(source, path);
    let parsed = Parser::with_dialect(source, resolved.dialect())
        .parse()
        .map_err(map_parse_error)?;

    let mut script = parsed.script;
    if resolved.simplify() || resolved.minify() {
        simplify::simplify_script(&mut script);
    }

    let comments = comments::Comments::from_ast(source, &parsed.comments);
    let context = context::ShellFormatContext::new(resolved, source, comments);
    let formatted = format!(context, [script.format()])
        .map_err(|error| FormatError::Internal(error.to_string()))?;
    let mut output = formatted
        .print()
        .map_err(|error| FormatError::Internal(error.to_string()))?
        .into_code();
    ensure_single_trailing_newline(&mut output);

    if output == source {
        Ok(FormattedSource::Unchanged)
    } else {
        Ok(FormattedSource::Formatted(output))
    }
}

fn ensure_single_trailing_newline(output: &mut String) {
    while output.ends_with("\n\n") {
        output.pop();
    }
    if !output.ends_with('\n') {
        output.push('\n');
    }
}

fn map_parse_error(error: ParseError) -> FormatError {
    match error {
        ParseError::Parse {
            message,
            line,
            column,
        } => FormatError::Parse {
            message,
            line,
            column,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_simple_command_with_tabs_by_default() {
        let formatted = format_source(
            "#!/bin/bash\n echo   hi\n",
            None,
            &ShellFormatOptions::default(),
        )
        .unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("#!/bin/bash\necho hi\n".to_string())
        );
    }

    #[test]
    fn preserves_inline_comments() {
        let formatted =
            format_source("echo hi    # note\n", None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("echo hi  # note\n".to_string())
        );
    }

    #[test]
    fn preserves_heredoc_commands_verbatim() {
        let source = "cat <<EOF\nhello\nEOF\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(formatted, FormattedSource::Unchanged);
    }

    #[test]
    fn honors_function_next_line_option() {
        let source = "foo(){\necho hi\n}\n";
        let options = ShellFormatOptions::default().with_function_next_line(true);
        let formatted = format_source(source, None, &options).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("foo()\n{\n\techo hi\n}\n".to_string())
        );
    }

    #[test]
    fn minify_drops_comments() {
        let options = ShellFormatOptions::default().with_minify(true);
        let formatted = format_source("#!/bin/bash\necho hi # note\n", None, &options).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("echo hi\n".to_string())
        );
    }

    #[test]
    fn formats_case_items_with_expected_indentation() {
        let source = "case $x in\na) echo a;;\nb) echo b;;\nesac\n";
        let formatted = format_source(source, None, &ShellFormatOptions::default()).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "case $x in\na) echo a ;;\nb) echo b ;;\nesac\n".to_string()
            )
        );
    }

    #[test]
    fn switch_case_indent_indents_patterns_and_bodies() {
        let source = "case $x in\na) echo a;;\nesac\n";
        let options = ShellFormatOptions::default().with_switch_case_indent(true);
        let formatted = format_source(source, None, &options).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("case $x in\n\ta)\n\t\techo a\n\t\t;;\nesac\n".to_string())
        );
    }

    #[test]
    fn space_redirects_insert_spaces_between_operator_and_target() {
        let options = ShellFormatOptions::default().with_space_redirects(true);
        let formatted = format_source("echo hi>out\n", None, &options).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("echo hi > out\n".to_string())
        );
    }

    #[test]
    fn keep_padding_preserves_safe_verbatim_regions() {
        let options = ShellFormatOptions::default().with_keep_padding(true);
        let formatted = format_source("a=1  b=2\n", None, &options).unwrap();

        assert_eq!(formatted, FormattedSource::Unchanged);
    }

    #[test]
    fn never_split_prefers_compact_layouts() {
        let options = ShellFormatOptions::default().with_never_split(true);
        let formatted = format_source("if true; then\necho hi\nfi\n", None, &options).unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted("if true; then echo hi; fi\n".to_string())
        );
    }

    #[test]
    fn auto_dialect_honors_env_shebang() {
        let error = format_source(
            "#!/usr/bin/env sh\n[[ foo == bar ]]\n",
            None,
            &ShellFormatOptions::default(),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            FormatError::Parse { message, .. } if message.contains("[[ ]] conditionals")
        ));
    }

    #[test]
    fn mksh_dialect_formats_select_commands() {
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Mksh);
        let formatted = format_source(
            "select name in foo; do echo \"$name\"; done\n",
            None,
            &options,
        )
        .unwrap();

        assert_eq!(
            formatted,
            FormattedSource::Formatted(
                "select name in foo; do\n\techo \"$name\";\ndone\n".to_string()
            )
        );
    }

    #[test]
    fn posix_dialect_propagates_parse_errors() {
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Posix);
        let error =
            format_source("[[ foo == bar ]]\n", Some(Path::new("test.sh")), &options).unwrap_err();

        assert!(matches!(
            error,
            FormatError::Parse { message, .. } if message.contains("[[ ]] conditionals")
        ));
    }
}
