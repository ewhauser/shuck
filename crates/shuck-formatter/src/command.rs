use crate::comments::SourceMap;
use crate::context::RenderContext;
use crate::facts::classify_stmt_contains_heredoc;
use crate::raw_syntax::{
    RawShellText, common_nonempty_shell_indent, leading_shell_indent,
    matching_raw_command_substitution_close, normalize_raw_pipeline_continuations,
    refine_common_indent, shell_comment_can_start, skip_escaped_or_quoted,
};
use crate::scan::{
    branch_keyword_offset, last_uncommented_shell_keyword_before, normalized_close_keyword_span,
};
use crate::word::{render_arithmetic_expr_to_buf, render_word_syntax_to_buf};
use shuck_ast::{
    AnonymousFunctionCommand, ArithmeticExprNode, ArithmeticForCommand, ArrayElem, Assignment,
    AssignmentValue, BackgroundOperator, BinaryCommand, BinaryOp, BuiltinCommand, CaseItem,
    CaseTerminator, Command, CompoundCommand, DeclClause, DeclOperand, ForCommand, ForSyntax,
    ForeachCommand, ForeachSyntax, FunctionDef, IfCommand, IfSyntax, RepeatCommand, RepeatSyntax,
    SelectCommand, SimpleCommand, SourceText, Span, Stmt, StmtSeq, StmtTerminator, Subscript,
    UntilCommand, VarRef, WhileCommand, Word, WordPart,
};
use shuck_indexer::CloseDelimiterKind;

mod arithmetic;
mod assignments;
mod body_site;
mod compound_assignments;
mod groups;
mod render_policy;
mod spans;
mod structure;
mod syntax;
mod traversal;

use self::groups::{
    command_group_attachment_span, find_empty_group_open_offset, group_verbatim_span_impl,
    stmt_group_base_span_with_heredoc,
};
use self::spans::{complete_stmt_span, span_for_offsets, stmt_verbatim_span_impl};
use self::traversal::{CompoundChild, for_each_compound_child};

pub(crate) use self::arithmetic::{
    format_arithmetic_command_source, format_arithmetic_for_clause_source,
};
pub(crate) use self::assignments::{
    array_elem_parts, array_elem_value_word_mut, render_assignment_head_to_buf,
    render_assignment_to_buf,
};
pub(crate) use self::body_site::{CompoundBodyOpen, CompoundBodySite};
pub(crate) use self::compound_assignments::{
    MultilineCompoundAssignmentLayout,
    multiline_compound_assignment_command_substitution_body_prefix,
    multiline_compound_assignment_layout, multiline_compound_assignment_lines,
};
pub(crate) use self::groups::{
    group_attachment_span, group_attachment_span_with_heredoc, group_open_suffix,
    group_was_inline_in_source, matching_group_close,
    stmt_group_attachment_or_verbatim_span_with_heredoc, stmt_start_after_operator,
};
pub(crate) use self::render_policy::{
    case_item_body_upper_bound, case_item_was_inline_in_source, if_close_span,
    line_gap_break_count, rendered_stmt_end_line_with_heredoc, should_render_verbatim_with_heredoc,
    stmt_attachment_span, stmt_attachment_span_with_heredoc, stmt_has_trailing_comment,
    stmt_render_start_line,
};
pub(crate) use self::spans::{
    anonymous_function_attachment_span, anonymous_function_header_span, builtin_like_parts,
    builtin_like_parts_mut, command_format_span, compound_format_span, function_attachment_span,
    function_header_span, merge_non_empty_span, simple_command_uses_synthetic_words,
    stmt_format_span, stmt_span, stmt_verbatim_span_with_source_map,
};
pub(crate) use self::structure::{
    branch_open_keyword_start, collect_binary_list_first, collect_pipeline_parts,
    command_group_commands, if_next_branch_region_with_body_end,
};
pub(crate) use self::syntax::{
    binary_operator, case_terminator, extend_heredoc_body_span, render_background_operator,
    render_subscript_to_buf, render_var_ref_to_buf, slice_span, trim_unescaped_trailing_whitespace,
};
