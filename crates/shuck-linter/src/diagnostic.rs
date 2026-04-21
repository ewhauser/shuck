use shuck_ast::Span;

use crate::{Fix, Rule, Violation};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Hint,
    Warning,
    Error,
}

impl Severity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hint => "hint",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub rule: Rule,
    pub message: String,
    pub severity: Severity,
    pub span: Span,
    pub fix: Option<Fix>,
    pub fix_title: Option<String>,
}

impl Diagnostic {
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

    pub const fn code(&self) -> &'static str {
        self.rule.code()
    }

    pub fn with_fix(mut self, fix: Fix) -> Self {
        self.fix = Some(fix);
        self
    }

    pub fn with_fix_title(mut self, fix_title: impl Into<String>) -> Self {
        self.fix_title = Some(fix_title.into());
        self
    }
}
