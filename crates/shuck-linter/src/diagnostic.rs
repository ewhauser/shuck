use shuck_ast::Span;

use crate::{Fix, Rule, Violation};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Advisory diagnostic that does not indicate likely incorrect behavior.
    Hint,
    /// Potential problem that warrants review.
    Warning,
    /// Definite error or invalid shell construct.
    Error,
}

impl Severity {
    /// Returns the lowercase name used by Shuck report formats.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hint => "hint",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// A diagnostic produced by Shuck analysis.
///
/// Fields remain public for downstream inspection, but consumers should not construct
/// diagnostics with struct literals.
///
/// ```compile_fail
/// use shuck_linter::{Diagnostic, Rule, Severity};
///
/// let _ = Diagnostic {
///     rule: Rule::UnusedAssignment,
///     message: String::new(),
///     severity: Severity::Warning,
///     span: todo!(),
///     fix: None,
///     fix_title: None,
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Diagnostic {
    /// Rule that emitted this diagnostic.
    pub rule: Rule,
    /// Human-readable explanation of the violation.
    pub message: String,
    /// Effective severity after applying configuration overrides.
    pub severity: Severity,
    /// Source span attributed to the diagnostic.
    ///
    /// Spans are defined by the `shuck-ast` crate and use byte offsets into the analyzed source.
    pub span: Span,
    /// Optional edit set that can correct the violation.
    pub fix: Option<Fix>,
    /// Optional human-readable label for the fix.
    pub fix_title: Option<String>,
}

impl Diagnostic {
    /// Creates a diagnostic from a rule-specific violation and source span.
    pub fn new<V: Violation>(violation: V, span: Span) -> Self {
        Self {
            rule: V::rule(),
            message: violation.message(),
            severity: V::rule().default_severity(),
            span,
            fix: None,
            fix_title: violation.fix_title(),
        }
    }

    /// Returns the stable rule code for this diagnostic.
    pub const fn code(&self) -> &'static str {
        self.rule.code()
    }

    /// Attaches an autofix to this diagnostic.
    pub fn with_fix(mut self, fix: Fix) -> Self {
        self.fix = Some(fix);
        self
    }
}
