use std::fmt::Write as _;

use crate::command::trim_unescaped_trailing_whitespace;
use crate::context::{FragmentFormatter, RenderContext};
use crate::facts::{FormatterFacts, classify_sequence_contains_multiline_literal_source};
use crate::options::{IndentStyle, ResolvedShellFormatOptions};
use crate::raw_syntax::{
    QuoteState, RawShellScanner, RawShellText, common_nonempty_shell_indent, heredoc_start,
    leading_shell_indent as line_leading_shell_indent, line_ends_with_raw_continuation_operator,
    line_without_continuation_backslash, matching_raw_command_substitution_close,
    normalize_raw_pipeline_continuations, redirect_operator_end, refine_common_indent,
};
use crate::scan::line_indent_before_offset as line_indent_before_source_offset;
use crate::source::{
    SourceView, command_substitution_source_closes_on_own_line,
    command_substitution_source_prefers_continued_inline_body,
    command_substitution_source_starts_with_body_line,
    dollar_command_substitution_body as raw_dollar_command_substitution_body,
    dollar_command_substitution_slice as raw_dollar_command_substitution_slice,
    substitution_source_closes_on_own_line,
};
use shuck_ast::{
    ArithmeticAssignOp, ArithmeticBinaryOp, ArithmeticExpansionSyntax, ArithmeticExpr,
    ArithmeticExprNode, ArithmeticLvalue, ArithmeticPostfixOp, ArithmeticUnaryOp, BinaryOp,
    BourneParameterExpansion, Command, CommandSubstitutionSyntax, CompoundCommand, HeredocBody,
    HeredocBodyPart, ParameterOp, Pattern, PatternPart, Stmt, StmtSeq, SubscriptSelector, VarRef,
    Word, WordPart, WordPartNode,
};

mod arithmetic;
mod command_substitution;
mod core;
mod heredoc;
mod parameter;
mod raw_rewrites;

pub(crate) use self::arithmetic::render_arithmetic_expr_to_buf;
pub(crate) use self::core::{
    render_escaped_multiline_word_syntax_to_buf, render_word_syntax_to_buf,
    word_gap_end_before_trailing_continuation, word_is_quoted_command_substitution_only,
    word_is_quoted_formattable_command_substitution_only_with_facts,
};
pub(crate) use self::heredoc::render_heredoc_body_to_buf;
pub(crate) use self::parameter::{parameter_defaulting_operator, render_pattern_syntax_to_buf};
pub(crate) use self::raw_rewrites::{
    normalize_raw_empty_parameter_replacement_delimiters, normalize_raw_unquoted_word_continuations,
};
