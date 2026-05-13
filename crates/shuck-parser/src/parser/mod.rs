//! Parser entrypoints, lexical types, and shell-profile configuration.
//!
//! The parser is recursive descent and produces `shuck-ast` syntax trees while also collecting
//! recovery diagnostics and lightweight syntax facts needed by downstream tooling.
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

mod arithmetic;
mod brace_syntax;
mod commands;
mod comments;
mod cursor;
mod diagnostics;
mod entry;
mod heredocs;
mod keywords;
mod lexer;
mod lowering;
mod parser_state;
mod profile;
mod recovery;
mod redirects;
mod result;
mod source_tree;
mod syntax_facts;
mod token_stream;
mod word_builders;
mod word_parts;
mod word_tokens;
mod words;
mod zsh_features;
mod zsh_options;
mod zsh_prescan;

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

pub use lexer::{LexedToken, Lexer};
pub(crate) use lexer::{LexedWordSegment, LexedWordSegmentKind};
pub use profile::{ShellDialect, ShellProfile};
pub use result::{ParseDiagnostic, ParseResult, ParseStatus, SyntaxFacts, ZshCaseGroupPart};
pub use zsh_options::{OptionValue, ZshEmulationMode, ZshOptionState};

use keywords::*;
use memchr::{memchr, memchr2, memchr3};
pub use parser_state::Parser;
#[cfg(feature = "benchmarking")]
pub use parser_state::ParserBenchmarkCounters;
use parser_state::*;
use smallvec::SmallVec;
use zsh_prescan::ZshOptionTimeline;

use shuck_ast::{
    AlwaysCommand, AnonymousFunctionCommand, AnonymousFunctionSurface, ArithmeticCommand,
    ArithmeticExpansionSyntax, ArithmeticExpr, ArithmeticExprNode, ArithmeticForCommand,
    ArithmeticLvalue, ArrayElem, ArrayExpr, ArrayKind, Assignment, AssignmentValue,
    BackgroundOperator, BinaryCommand, BinaryOp, BourneParameterExpansion, BraceExpansionKind,
    BraceQuoteContext, BraceSyntax, BraceSyntaxKind, BreakCommand as AstBreakCommand,
    BuiltinCommand as AstBuiltinCommand, CaseCommand, CaseItem, CaseTerminator,
    Command as AstCommand, CommandSubstitutionSyntax, Comment, CompoundCommand,
    ConditionalBinaryExpr, ConditionalBinaryOp, ConditionalCommand, ConditionalExpr,
    ConditionalParenExpr, ConditionalUnaryExpr, ConditionalUnaryOp,
    ContinueCommand as AstContinueCommand, CoprocCommand, DeclClause as AstDeclClause, DeclOperand,
    ExitCommand as AstExitCommand, File, ForCommand, ForSyntax, ForTarget, ForeachCommand,
    ForeachSyntax, FunctionDef, FunctionHeader, FunctionHeaderEntry, Heredoc, HeredocBody,
    HeredocBodyMode, HeredocBodyPart, HeredocBodyPartNode, HeredocDelimiter, IfCommand, IfSyntax,
    LiteralText, Name, ParameterExpansion, ParameterExpansionSyntax, ParameterOp, Pattern,
    PatternGroupKind, PatternPart, PatternPartNode, Position, PrefixMatchKind, Redirect,
    RedirectKind, RedirectTarget, RepeatCommand, RepeatSyntax, ReturnCommand as AstReturnCommand,
    SelectCommand, SimpleCommand as AstSimpleCommand, SourceText, Span, Stmt, StmtSeq,
    StmtTerminator, Subscript, SubscriptInterpretation, SubscriptKind, SubscriptSelector, TextSize,
    TimeCommand, TokenKind, UntilCommand, VarRef, WhileCommand, Word, WordPart, WordPartNode,
    ZshDefaultingOp, ZshExpansionOperation, ZshExpansionTarget, ZshGlobQualifier,
    ZshGlobQualifierGroup, ZshGlobQualifierKind, ZshGlobSegment, ZshInlineGlobControl, ZshModifier,
    ZshParameterExpansion, ZshPatternOp, ZshQualifiedGlob, ZshReplacementOp, ZshTrimOp,
};

use crate::error::{Error, Result};

type WordPartBuffer = SmallVec<[WordPartNode; 2]>;

#[derive(Debug, Clone, Copy, Default)]
struct ZshGlobParseFeatures {
    classic_qualifiers: bool,
    extended_glob: bool,
    ksh_groups: bool,
    bare_groups: bool,
}

impl ZshGlobParseFeatures {
    const fn zsh_word_parsing_enabled(self) -> bool {
        self.classic_qualifiers || self.extended_glob || self.ksh_groups || self.bare_groups
    }
}

/// Default maximum AST depth (matches ExecutionLimits default)
const DEFAULT_MAX_AST_DEPTH: usize = 100;

/// Hard cap on AST depth to prevent stack overflow even if caller misconfigures limits.
/// Protects against deeply nested input attacks where
/// a large max_depth setting allows recursion deep enough to overflow the native stack.
/// This cap cannot be overridden by the caller.
///
/// Set conservatively to avoid stack overflow on tokio's blocking threads (default 2MB
/// stack in debug builds). Each parser recursion level uses ~4-8KB of stack in debug
/// mode. 100 levels × ~8KB = ~800KB, well within 2MB.
/// In release builds this could safely be higher, but we use one value for consistency.
const HARD_MAX_AST_DEPTH: usize = 100;

/// Auxiliary word reparsing happens while the main parser is already on the stack.
/// Keep its synthetic parser shallower than the main AST limit.
const SOURCE_TEXT_WORD_REPARSE_MAX_DEPTH: usize = 8;

/// Pattern operands can themselves contain parameter expansions with pattern operands.
/// Keep that source-text reparsing shallow and preserve deeper text literally.
const SOURCE_TEXT_PATTERN_REPARSE_MAX_DEPTH: usize = 4;

/// Default maximum parser operations (matches ExecutionLimits default)
const DEFAULT_MAX_PARSER_OPERATIONS: usize = 100_000;

/// Returns whether `text` parses as a nontrivial arithmetic expression.
///
/// Plain numbers and plain variable names are considered trivial. The helper
/// returns `false` for empty text and for text that cannot be parsed inside a
/// shell arithmetic command.
pub fn text_looks_like_nontrivial_arithmetic_expression(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }

    let source = format!("(( {text} ))");
    let file = Parser::new(&source).parse();
    if file.is_err() {
        return false;
    }

    let Some(statement) = file.file.body.first() else {
        return false;
    };

    let AstCommand::Compound(CompoundCommand::Arithmetic(command)) = &statement.command else {
        return false;
    };

    command.expr_ast.as_ref().is_some_and(|expr| {
        !matches!(
            expr.kind,
            ArithmeticExpr::Number(_) | ArithmeticExpr::Variable(_)
        )
    })
}

/// Returns whether `text` parses as an arithmetic expression without variable
/// references, subscripts, shell words, or assignments.
///
/// This is useful when a caller needs a purely self-contained arithmetic value.
/// Invalid or empty text returns `false`.
pub fn text_is_self_contained_arithmetic_expression(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }

    let source = format!("(( {text} ))");
    let file = Parser::new(&source).parse();
    if file.is_err() {
        return false;
    }

    let Some(statement) = file.file.body.first() else {
        return false;
    };

    let AstCommand::Compound(CompoundCommand::Arithmetic(command)) = &statement.command else {
        return false;
    };

    command
        .expr_ast
        .as_ref()
        .is_some_and(arithmetic_expr_is_self_contained)
}

fn arithmetic_expr_is_self_contained(expr: &ArithmeticExprNode) -> bool {
    match &expr.kind {
        ArithmeticExpr::Number(_) => true,
        ArithmeticExpr::Variable(_)
        | ArithmeticExpr::Indexed { .. }
        | ArithmeticExpr::ShellWord(_)
        | ArithmeticExpr::Assignment { .. } => false,
        ArithmeticExpr::Parenthesized { expression } => {
            arithmetic_expr_is_self_contained(expression)
        }
        ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
            arithmetic_expr_is_self_contained(expr)
        }
        ArithmeticExpr::Binary { left, right, .. } => {
            arithmetic_expr_is_self_contained(left) && arithmetic_expr_is_self_contained(right)
        }
        ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            arithmetic_expr_is_self_contained(condition)
                && arithmetic_expr_is_self_contained(then_expr)
                && arithmetic_expr_is_self_contained(else_expr)
        }
    }
}

#[cfg(test)]
mod arithmetic_text_helper_tests {
    use super::{
        text_is_self_contained_arithmetic_expression,
        text_looks_like_nontrivial_arithmetic_expression,
    };

    #[test]
    fn requires_nontrivial_expressions() {
        assert!(text_looks_like_nontrivial_arithmetic_expression("1 + 2"));
        assert!(text_looks_like_nontrivial_arithmetic_expression("arr[1]"));
        assert!(text_looks_like_nontrivial_arithmetic_expression("++count"));
        assert!(!text_looks_like_nontrivial_arithmetic_expression("123"));
        assert!(!text_looks_like_nontrivial_arithmetic_expression("name"));
        assert!(!text_looks_like_nontrivial_arithmetic_expression(
            "latest value"
        ));
    }

    #[test]
    fn distinguishes_self_contained_expressions() {
        assert!(text_is_self_contained_arithmetic_expression("1 + 2"));
        assert!(text_is_self_contained_arithmetic_expression("(1 + 2)"));
        assert!(!text_is_self_contained_arithmetic_expression("name"));
        assert!(!text_is_self_contained_arithmetic_expression("arr[1]"));
        assert!(!text_is_self_contained_arithmetic_expression("foo + 1"));
        assert!(!text_is_self_contained_arithmetic_expression(
            "latest value"
        ));
    }
}

#[cfg(test)]
mod tests;
