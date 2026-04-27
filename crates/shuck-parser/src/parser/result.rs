use shuck_ast::{File, Span};

use crate::error::Error;

/// Overall outcome of a parse attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseStatus {
    /// The parse completed without recovery diagnostics.
    Clean,
    /// The parse completed, but required recovery diagnostics.
    Recovered,
    /// The parse failed with a terminal error.
    Fatal,
}

/// One branch separator recognized inside a zsh `case` pattern group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZshCaseGroupPart {
    /// Index of the owning pattern part within the parsed pattern.
    pub pattern_part_index: usize,
    /// Source span covering the separator syntax.
    pub span: Span,
}

/// Additional parser-owned facts that are useful to downstream consumers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyntaxFacts {
    /// Spans of zsh brace-style `if` bodies.
    pub zsh_brace_if_spans: Vec<Span>,
    /// Spans of zsh `always` clauses.
    pub zsh_always_spans: Vec<Span>,
    /// Pattern-group separators collected from zsh `case` items.
    pub zsh_case_group_parts: Vec<ZshCaseGroupPart>,
}

/// A parser diagnostic emitted while recovering from invalid input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDiagnostic {
    /// Human-readable diagnostic message.
    pub message: String,
    /// Source span associated with the diagnostic.
    pub span: Span,
}

/// The result of parsing a script, including any recovery diagnostics and
/// syntax facts collected along the way.
#[derive(Debug, Clone)]
pub struct ParseResult {
    /// Parsed syntax tree for the file.
    pub file: File,
    /// Recovery diagnostics emitted while producing the AST.
    pub diagnostics: Vec<ParseDiagnostic>,
    /// High-level parse status.
    pub status: ParseStatus,
    /// Terminal parse error, when recovery could not continue.
    pub terminal_error: Option<Error>,
    /// Additional syntax facts collected during parsing.
    pub syntax_facts: SyntaxFacts,
}

impl ParseResult {
    /// Returns `true` when the parse completed without recovery diagnostics.
    pub fn is_ok(&self) -> bool {
        self.status == ParseStatus::Clean
    }

    /// Returns `true` when the parse produced recovery diagnostics or a terminal error.
    pub fn is_err(&self) -> bool {
        !self.is_ok()
    }

    /// Convert this result into a strict parse error.
    ///
    /// If recovery diagnostics exist but no terminal error was recorded, the first recovery
    /// diagnostic is converted into an [`Error`].
    pub fn strict_error(&self) -> Error {
        self.terminal_error.clone().unwrap_or_else(|| {
            let Some(diagnostic) = self.diagnostics.first() else {
                panic!("non-clean parse result should include a diagnostic or terminal error");
            };
            Error::parse_at(
                diagnostic.message.clone(),
                diagnostic.span.start.line,
                diagnostic.span.start.column,
            )
        })
    }

    /// Return the parse result when it is clean, otherwise panic with the strict error.
    pub fn unwrap(self) -> Self {
        if self.is_ok() {
            self
        } else {
            panic!(
                "called `ParseResult::unwrap()` on a non-clean parse: {}",
                self.strict_error()
            )
        }
    }

    /// Return the parse result when it is clean, otherwise panic with `message`.
    pub fn expect(self, message: &str) -> Self {
        if self.is_ok() {
            self
        } else {
            panic!("{message}: {}", self.strict_error())
        }
    }

    /// Return the strict parse error when the result is not clean, otherwise panic.
    pub fn unwrap_err(self) -> Error {
        if self.is_err() {
            self.strict_error()
        } else {
            panic!("called `ParseResult::unwrap_err()` on a clean parse")
        }
    }

    /// Return the strict parse error when the result is not clean, otherwise panic with
    /// `message`.
    pub fn expect_err(self, message: &str) -> Error {
        if self.is_err() {
            self.strict_error()
        } else {
            panic!("{message}")
        }
    }
}
