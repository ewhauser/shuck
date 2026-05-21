#![warn(missing_docs)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]
//! Shell formatting entrypoints built on top of `shuck-parser`.
//!
//! Most callers will use [`format_source`] for source text or [`format_file_ast`] when they
//! already have a parsed shell AST.
#![recursion_limit = "256"]

//! Shell script formatter with configurable style options.

#[allow(missing_docs)]
mod command;
#[allow(missing_docs)]
mod comments;
mod facts;
#[allow(missing_docs)]
mod options;
#[allow(missing_docs)]
mod scan;
#[allow(missing_docs)]
mod simplify;
#[allow(missing_docs)]
mod streaming;
#[allow(missing_docs)]
mod visit;
#[allow(missing_docs)]
mod word;

use std::path::Path;

use shuck_ast::File;
use shuck_parser::{Error as ParseError, parser::Parser};

use crate::facts::FormatterFacts;

/// Formatter option types exposed by the shell formatter.
pub use crate::options::{
    IndentStyle, LineEnding, ResolvedShellFormatOptions, ShellDialect, ShellFormatOptions,
};

/// Result of formatting shell source.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormattedSource {
    Unchanged,
    Formatted(String),
}

#[allow(missing_docs)]
impl FormattedSource {
    #[must_use]
    pub fn is_changed(&self) -> bool {
        matches!(self, Self::Formatted(_))
    }
}

/// Errors that can occur while parsing or formatting shell source.
#[allow(missing_docs)]
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

/// Convenient result alias for shell formatting operations.
pub type Result<T> = std::result::Result<T, FormatError>;

/// Formats a shell source string using the provided options.
pub fn format_source(
    source: &str,
    path: Option<&Path>,
    options: &ShellFormatOptions,
) -> Result<FormattedSource> {
    let resolved = options.resolve_for_format(source, path);

    let dialect = resolved.dialect();
    let parsed = Parser::with_dialect(source, dialect).parse();
    if parsed.is_err() {
        return Err(map_parse_error(parsed.strict_error()));
    }

    format_file_ast(source, parsed.file, path, options)
}

#[doc(hidden)]
pub fn source_is_formatted(
    source: &str,
    path: Option<&Path>,
    options: &ShellFormatOptions,
) -> Result<bool> {
    let resolved = options.resolve_for_format(source, path);

    let dialect = resolved.dialect();
    let parsed = Parser::with_dialect(source, dialect).parse();
    if parsed.is_err() {
        return Err(map_parse_error(parsed.strict_error()));
    }

    check_file(source, parsed.file, resolved)
}

/// Formats a parsed shell file using the provided options.
pub fn format_file_ast(
    source: &str,
    file: File,
    path: Option<&Path>,
    options: &ShellFormatOptions,
) -> Result<FormattedSource> {
    let resolved = options.resolve_for_format(source, path);
    let output = format_output(source, file, &resolved)?;

    Ok(formatted_source_from_output(source, output))
}

fn check_file(source: &str, mut file: File, resolved: ResolvedShellFormatOptions) -> Result<bool> {
    if resolved.minify() {
        let output = format_output(source, file, &resolved)?;
        return Ok(output == source);
    }

    if resolved.simplify() {
        simplify::simplify_file(&mut file, source);
    }

    let facts = FormatterFacts::build(source, &file, &resolved);
    let resolved = resolved.with_line_ending(facts.line_ending());
    streaming::format_file_streaming_matches_source_with_facts(source, &file, &resolved, &facts)
}

fn format_output(
    source: &str,
    mut file: File,
    resolved: &ResolvedShellFormatOptions,
) -> Result<String> {
    if resolved.simplify() || resolved.minify() {
        simplify::simplify_file(&mut file, source);
    }

    let facts = FormatterFacts::build(source, &file, resolved);
    let resolved = resolved.clone().with_line_ending(facts.line_ending());
    let mut output = streaming::format_file_streaming_with_facts(source, &file, &resolved, &facts)?;
    if resolved.minify() {
        preserve_initial_shebang(source, &mut output, resolved.line_ending());
    }
    ensure_single_trailing_newline(&mut output, resolved.line_ending());

    Ok(output)
}

fn formatted_source_from_output(source: &str, output: String) -> FormattedSource {
    if output == source {
        FormattedSource::Unchanged
    } else {
        FormattedSource::Formatted(output)
    }
}

#[cfg(feature = "benchmarking")]
#[doc(hidden)]
#[must_use]
pub fn build_formatter_facts(source: &str, file: &File) -> usize {
    let resolved = ShellFormatOptions::default().resolve_for_format(source, None);
    FormatterFacts::build(source, file, &resolved).len()
}

fn ensure_single_trailing_newline(output: &mut String, line_ending: LineEnding) {
    while let Some(start) = trailing_line_ending_start(output)
        .filter(|start| trailing_line_ending_start(&output[..*start]).is_some())
    {
        output.truncate(start);
    }
    if trailing_line_ending_start(output).is_none() {
        if trailing_backslash_count(output) % 2 == 1 && !trailing_backslash_is_in_comment(output) {
            output.push('\\');
        }
        output.push_str(line_ending_str(line_ending));
    }
}

fn trailing_line_ending_start(text: &str) -> Option<usize> {
    if text.ends_with("\r\n") {
        Some(text.len() - 2)
    } else if text.ends_with('\n') {
        Some(text.len() - 1)
    } else {
        None
    }
}

fn line_ending_str(line_ending: LineEnding) -> &'static str {
    match line_ending {
        LineEnding::Lf => "\n",
        LineEnding::CrLf => "\r\n",
    }
}

fn preserve_initial_shebang(source: &str, output: &mut String, line_ending: LineEnding) {
    if !source.starts_with("#!") || output.starts_with("#!") {
        return;
    }

    let shebang_end = source.find(['\r', '\n']).unwrap_or(source.len());
    let shebang = &source[..shebang_end];
    let line_ending = line_ending_str(line_ending);
    let body = output.trim_start_matches(['\r', '\n']);

    let mut prefixed = String::with_capacity(shebang.len() + line_ending.len() + body.len());
    prefixed.push_str(shebang);
    prefixed.push_str(line_ending);
    prefixed.push_str(body);
    *output = prefixed;
}

fn trailing_backslash_count(text: &str) -> usize {
    text.as_bytes()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
}

fn trailing_backslash_is_in_comment(text: &str) -> bool {
    let line = text.rsplit_once('\n').map_or(text, |(_, line)| line);
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut escaped = false;

    for (index, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if !in_single_quotes => {
                escaped = true;
            }
            '\'' if !in_double_quotes => {
                in_single_quotes = !in_single_quotes;
            }
            '"' if !in_single_quotes => {
                in_double_quotes = !in_double_quotes;
            }
            '#' if !in_single_quotes
                && !in_double_quotes
                && scan::shell_comment_can_start(line, index) =>
            {
                return true;
            }
            _ => {}
        }
    }

    false
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
