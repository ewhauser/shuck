//! Owned syntax tree types for GitHub Actions expressions.

use std::fmt::{Debug, Formatter};

use crate::{Name, TextRange, TextSize};

/// A source-preserving GitHub Actions template embedded in a decoded YAML scalar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubTemplate {
    /// Alternating literal and expression segments covering [`Self::range`].
    pub segments: Vec<GitHubTemplateSegment>,
    /// Complete byte range of the decoded template source.
    pub range: TextRange,
}

/// One source-backed segment in a GitHub Actions template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubTemplateSegment {
    /// Literal text passed through to the shell after expression interpolation.
    Literal {
        /// Byte range in the decoded template source.
        range: TextRange,
    },
    /// A `${{ ... }}` expression evaluated before the shell runs.
    Expression(GitHubTemplateExpression),
}

/// One parsed `${{ ... }}` segment in a GitHub Actions template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubTemplateExpression {
    /// Full byte range including `${{` and `}}`.
    pub range: TextRange,
    /// Trimmed expression-body byte range.
    pub body_range: TextRange,
    /// Parsed expression, or a recoverable expression-local diagnostic.
    pub parsed: GitHubExpressionParse,
}

/// The result of parsing one `${{ ... }}` expression body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubExpressionParse {
    /// The expression parsed successfully.
    Parsed(GitHubExpressionNode),
    /// The expression was malformed, but its surrounding template remains usable.
    Invalid(GitHubExpressionDiagnostic),
}

impl GitHubExpressionParse {
    /// Shift every stored range by `base` bytes.
    pub fn offset_by(&mut self, base: TextSize) {
        match self {
            Self::Parsed(node) => node.offset_by(base),
            Self::Invalid(diagnostic) => {
                diagnostic.range = diagnostic.range.offset_by(base);
            }
        }
    }
}

/// A syntax error within a GitHub Actions expression body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubExpressionDiagnostic {
    /// Stable classification of the parse failure.
    pub kind: GitHubExpressionDiagnosticKind,
    /// Human-readable description authored by the expression parser adapter.
    pub message: String,
    /// Byte range of the offending syntax, or an empty range at the error position.
    pub range: TextRange,
}

/// Stable categories of GitHub Actions expression parse failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubExpressionDiagnosticKind {
    /// The expression does not conform to the expression grammar.
    Syntax,
    /// The expression calls a function that GitHub Actions does not recognize.
    UnknownFunction {
        /// Function name as written in the expression.
        function: Box<str>,
    },
    /// A built-in function received an unsupported argument count.
    InvalidFunctionArguments {
        /// Function whose argument count is invalid.
        function: GitHubExpressionFunction,
    },
}

/// A source-backed GitHub Actions expression node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubExpressionNode {
    /// Byte range in the tree's backing source.
    pub range: TextRange,
    /// The node's syntactic form.
    pub kind: GitHubExpressionKind,
}

impl GitHubExpressionNode {
    /// Return the source text covered by this node.
    pub fn source<'a>(&self, expression: &'a str) -> &'a str {
        self.range.slice(expression)
    }

    /// Shift this node and all descendants by `base` bytes.
    pub fn offset_by(&mut self, base: TextSize) {
        self.range = self.range.offset_by(base);
        match &mut self.kind {
            GitHubExpressionKind::Literal(_)
            | GitHubExpressionKind::Star
            | GitHubExpressionKind::Identifier(_) => {}
            GitHubExpressionKind::Grouped { expression }
            | GitHubExpressionKind::Index(expression) => expression.offset_by(base),
            GitHubExpressionKind::Call { arguments, .. }
            | GitHubExpressionKind::Context(arguments) => {
                for argument in arguments {
                    argument.offset_by(base);
                }
            }
            GitHubExpressionKind::Binary {
                left,
                operator_range,
                right,
                ..
            } => {
                left.offset_by(base);
                *operator_range = operator_range.offset_by(base);
                right.offset_by(base);
            }
            GitHubExpressionKind::Unary {
                operator_range,
                expression,
                ..
            } => {
                *operator_range = operator_range.offset_by(base);
                expression.offset_by(base);
            }
        }
    }
}

/// Syntactic forms in the GitHub Actions expression language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubExpressionKind {
    /// A literal value.
    Literal(GitHubExpressionLiteral),
    /// The wildcard used in an object filter or index.
    Star,
    /// An expression surrounded by parentheses.
    Grouped {
        /// Expression inside the parentheses.
        expression: Box<GitHubExpressionNode>,
    },
    /// A built-in function call.
    Call {
        /// Built-in function being called.
        function: GitHubExpressionFunction,
        /// Function arguments in source order.
        arguments: Vec<GitHubExpressionNode>,
    },
    /// One identifier component.
    Identifier(Name),
    /// A computed or literal bracket index.
    Index(Box<GitHubExpressionNode>),
    /// A complete context or value-access chain.
    Context(Vec<GitHubExpressionNode>),
    /// A binary expression.
    Binary {
        /// Left operand.
        left: Box<GitHubExpressionNode>,
        /// Binary operator.
        operator: GitHubExpressionBinaryOperator,
        /// Byte range of the operator token.
        operator_range: TextRange,
        /// Right operand.
        right: Box<GitHubExpressionNode>,
    },
    /// A unary expression.
    Unary {
        /// Unary operator.
        operator: GitHubExpressionUnaryOperator,
        /// Byte range of the operator token.
        operator_range: TextRange,
        /// Operand.
        expression: Box<GitHubExpressionNode>,
    },
}

/// Literal values supported by GitHub Actions expressions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubExpressionLiteral {
    /// Parsed numeric value. The node range retains its original spelling.
    Number(GitHubExpressionNumber),
    /// A cooked string value with doubled quote escapes decoded.
    String(Box<str>),
    /// A boolean value.
    Boolean(bool),
    /// The null value.
    Null,
}

/// An IEEE-754 numeric value parsed from a GitHub Actions expression.
///
/// Zero and NaN values are canonicalized so structural equality remains reflexive while the
/// original literal spelling remains available through the containing node's source range.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct GitHubExpressionNumber(u64);

impl GitHubExpressionNumber {
    /// Construct a numeric literal value.
    pub fn new(value: f64) -> Self {
        let canonical = if value == 0.0 {
            0.0
        } else if value.is_nan() {
            f64::NAN
        } else {
            value
        };
        Self(canonical.to_bits())
    }

    /// Return the numeric value.
    pub fn value(self) -> f64 {
        f64::from_bits(self.0)
    }
}

impl Debug for GitHubExpressionNumber {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("GitHubExpressionNumber")
            .field(&self.value())
            .finish()
    }
}

/// Built-in functions supported by GitHub Actions expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GitHubExpressionFunction {
    /// Test whether one value contains another.
    Contains,
    /// Test whether a string starts with a prefix.
    StartsWith,
    /// Test whether a string ends with a suffix.
    EndsWith,
    /// Interpolate values into a format string.
    Format,
    /// Join an array into a string.
    Join,
    /// Serialize a value as JSON.
    ToJson,
    /// Parse a JSON value.
    FromJson,
    /// Hash one or more workspace file patterns.
    HashFiles,
    /// Select a value from predicate/value pairs.
    Case,
    /// Test whether earlier workflow work succeeded.
    Success,
    /// Always evaluate as successful for status gating.
    Always,
    /// Test whether the workflow was cancelled.
    Cancelled,
    /// Test whether earlier workflow work failed.
    Failure,
}

/// Binary operators supported by GitHub Actions expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitHubExpressionBinaryOperator {
    /// Logical conjunction (`&&`).
    And,
    /// Logical disjunction (`||`).
    Or,
    /// Equality (`==`).
    Equal,
    /// Inequality (`!=`).
    NotEqual,
    /// Greater than (`>`).
    GreaterThan,
    /// Greater than or equal (`>=`).
    GreaterThanOrEqual,
    /// Less than (`<`).
    LessThan,
    /// Less than or equal (`<=`).
    LessThanOrEqual,
}

/// Unary operators supported by GitHub Actions expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitHubExpressionUnaryOperator {
    /// Logical negation (`!`).
    Not,
}

#[cfg(test)]
mod tests {
    use super::GitHubExpressionNumber;

    #[test]
    fn expression_numbers_have_reflexive_structural_equality() {
        let nan = GitHubExpressionNumber::new(f64::NAN);
        assert_eq!(nan, nan);
        assert!(nan.value().is_nan());

        assert_eq!(
            GitHubExpressionNumber::new(-0.0),
            GitHubExpressionNumber::new(0.0)
        );
    }
}
