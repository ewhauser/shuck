use std::path::Path;

use shuck_parser::Error as ParseError;
use shuck_parser::parser::Parser;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndentStyle {
    Space,
    Tab,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteStyle {
    Preserve,
    Single,
    Double,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Auto,
    Lf,
    CrLf,
    Native,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatterSettings {
    pub line_width: u16,
    pub indent_style: IndentStyle,
    pub quote_style: QuoteStyle,
    pub line_ending: LineEnding,
}

impl Default for FormatterSettings {
    fn default() -> Self {
        Self {
            line_width: 88,
            indent_style: IndentStyle::Space,
            quote_style: QuoteStyle::Preserve,
            line_ending: LineEnding::Auto,
        }
    }
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
        }
    }
}

impl std::error::Error for FormatError {}

pub type Result<T> = std::result::Result<T, FormatError>;

pub fn format_source(
    source: &str,
    path: Option<&Path>,
    settings: &FormatterSettings,
) -> Result<FormattedSource> {
    let _parsed = Parser::new(source).parse().map_err(map_parse_error)?;
    Ok(format_script(source, path, settings))
}

fn format_script(
    _source: &str,
    _path: Option<&Path>,
    _settings: &FormatterSettings,
) -> FormattedSource {
    FormattedSource::Unchanged
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
    fn valid_input_is_unchanged() {
        let formatted = format_source(
            "#!/bin/bash\necho ok\n",
            None,
            &FormatterSettings::default(),
        )
        .unwrap();

        assert_eq!(formatted, FormattedSource::Unchanged);
    }

    #[test]
    fn invalid_input_reports_parse_error() {
        let error = format_source(
            "#!/bin/bash\nif true\n",
            None,
            &FormatterSettings::default(),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            FormatError::Parse {
                line: 2,
                column: _,
                ..
            }
        ));
    }
}
