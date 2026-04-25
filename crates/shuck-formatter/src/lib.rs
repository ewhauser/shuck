#![warn(missing_docs)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]
//! Shell formatting entrypoints.
//!
//! The previous formatter implementation has been removed so the formatter can
//! be rebuilt from stubs.

#[allow(missing_docs)]
mod options;

use std::path::Path;

/// Formatter option types exposed by the shell formatter.
pub use crate::options::{
    IndentStyle, LineEnding, ResolvedShellFormatOptions, ShellDialect, ShellFormatOptions,
};

/// Result of formatting shell source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormattedSource {
    /// The formatter did not produce a replacement for the source.
    Unchanged,
    /// The formatter produced replacement source.
    Formatted(String),
}

impl FormattedSource {
    /// Returns true when this result contains replacement source.
    #[must_use]
    pub fn is_changed(&self) -> bool {
        matches!(self, Self::Formatted(_))
    }
}

/// Errors that can occur while parsing or formatting shell source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatError {
    /// Parsing failed before formatting could begin.
    Parse {
        /// Parser error message.
        message: String,
        /// 1-based source line when available.
        line: usize,
        /// 1-based source column when available.
        column: usize,
    },
    /// Placeholder for formatter-internal errors once the implementation is rebuilt.
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
///
/// This stub returns [`FormattedSource::Unchanged`] without inspecting the source.
pub fn format_source(
    _source: &str,
    _path: Option<&Path>,
    _options: &ShellFormatOptions,
) -> Result<FormattedSource> {
    Ok(FormattedSource::Unchanged)
}

#[doc(hidden)]
pub fn source_is_formatted(
    _source: &str,
    _path: Option<&Path>,
    _options: &ShellFormatOptions,
) -> Result<bool> {
    Ok(true)
}

/// Formats a parsed shell file using the provided options.
///
/// This stub preserves the API used by callers while avoiding an AST dependency.
pub fn format_file_ast<File>(
    _source: &str,
    _file: File,
    _path: Option<&Path>,
    _options: &ShellFormatOptions,
) -> Result<FormattedSource> {
    Ok(FormattedSource::Unchanged)
}

#[cfg(feature = "benchmarking")]
#[doc(hidden)]
#[must_use]
pub fn build_formatter_facts<File>(_source: &str, _file: &File) -> usize {
    0
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn format_source_is_currently_a_noop() {
        let formatted = format_source(
            " echo   hi\n",
            Some(Path::new("script.sh")),
            &ShellFormatOptions::default(),
        )
        .unwrap();

        assert_eq!(formatted, FormattedSource::Unchanged);
        assert!(
            source_is_formatted(
                " echo   hi\n",
                Some(Path::new("script.sh")),
                &ShellFormatOptions::default()
            )
            .unwrap()
        );
    }
}
