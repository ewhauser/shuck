//! Error types for shuck

/// Result type alias using shuck's Error.
pub type Result<T> = std::result::Result<T, Error>;

/// Shuck error types.
#[derive(Debug)]
pub enum Error {
    /// Parse error occurred while parsing the script.
    ///
    /// When `line` and `column` are 0, the error has no source location.
    Parse {
        message: String,
        line: usize,
        column: usize,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self::Parse {
            message,
            line,
            column,
        } = self;
        if *line > 0 {
            write!(f, "parse error at line {line}, column {column}: {message}")
        } else {
            write!(f, "parse error: {message}")
        }
    }
}

impl std::error::Error for Error {}

impl Error {
    /// Create a parse error with source location.
    pub fn parse_at(message: impl Into<String>, line: usize, column: usize) -> Self {
        Self::Parse {
            message: message.into(),
            line,
            column,
        }
    }

    /// Create a parse error without source location.
    pub fn parse(message: impl Into<String>) -> Self {
        Self::Parse {
            message: message.into(),
            line: 0,
            column: 0,
        }
    }
}
