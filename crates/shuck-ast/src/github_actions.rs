//! Owned syntax tree types for GitHub Actions expressions.

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

impl GitHubTemplateSegment {
    /// Return the segment's byte range in the decoded template source.
    pub fn range(&self) -> TextRange {
        match self {
            Self::Literal { range } => *range,
            Self::Expression(expression) => expression.range,
        }
    }
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
    /// Return the parsed root node, if parsing succeeded.
    pub fn as_node(&self) -> Option<&GitHubExpressionNode> {
        match self {
            Self::Parsed(node) => Some(node),
            Self::Invalid(_) => None,
        }
    }

    /// Return the parse diagnostic, if parsing failed.
    pub fn as_diagnostic(&self) -> Option<&GitHubExpressionDiagnostic> {
        match self {
            Self::Parsed(_) => None,
            Self::Invalid(diagnostic) => Some(diagnostic),
        }
    }

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
    /// Description returned by the expression parser adapter.
    pub message: String,
    /// Empty byte range at the error position in the tree's backing source.
    pub range: TextRange,
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
        /// Canonical lowercase function name.
        function: Name,
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
    /// A normalized numeric value. The node range retains its original spelling.
    Number(Name),
    /// A cooked string value with doubled quote escapes decoded.
    String(Name),
    /// A boolean value.
    Boolean(bool),
    /// The null value.
    Null,
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
