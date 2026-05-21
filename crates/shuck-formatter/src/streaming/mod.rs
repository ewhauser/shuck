use std::fmt::Write as _;
use std::mem;

use shuck_ast::{
    AlwaysCommand, AnonymousFunctionCommand, ArithmeticCommand, ArithmeticForCommand, ArrayElem,
    Assignment, AssignmentValue, BinaryCommand, BinaryOp, BuiltinCommand, CaseCommand, CaseItem,
    Command, CompoundCommand, ConditionalBinaryExpr, ConditionalBinaryOp, ConditionalCommand,
    ConditionalExpr, ConditionalParenExpr, ConditionalUnaryExpr, ConditionalUnaryOp, CoprocCommand,
    DeclClause, DeclOperand, File, ForCommand, ForSyntax, ForeachCommand, ForeachSyntax,
    FunctionDef, HeredocBody, HeredocBodyPart, IfCommand, IfSyntax, Pattern, PatternPart, Redirect,
    RedirectKind, RepeatCommand, RepeatSyntax, SelectCommand, SimpleCommand, Span, Stmt, StmtSeq,
    StmtTerminator, TimeCommand, UntilCommand, VarRef, WhileCommand, Word, WordPart,
};

mod branches;
mod case_layout;
mod comments_alignment;
mod compounds;
mod conditions;
mod gaps;
mod redirects;
mod shape;
mod statements;
mod substitutions;
mod writer;

use crate::Result;
use crate::command::{
    array_elem_parts, binary_operator, branch_open_keyword_start, builtin_like_parts,
    case_item_body_upper_bound, case_terminator,
    collect_binary_list_first as collect_binary_list_first_with, collect_pipeline_parts,
    command_format_span, command_group_commands, done_close_span as command_done_close_span,
    format_arithmetic_command_source, format_arithmetic_for_clause_source, group_attachment_span,
    if_close_span as command_if_close_span, if_next_branch_region_with_body_end,
    line_gap_break_count, matching_group_close,
    multiline_compound_assignment_command_substitution_body_prefix,
    multiline_compound_assignment_layout, multiline_compound_assignment_lines,
    render_assignment_head_to_buf, render_assignment_to_buf, render_background_operator,
    render_subscript_to_buf, render_var_ref_to_buf, simple_command_uses_synthetic_words,
    slice_span, stmt_attachment_span, stmt_format_span, stmt_render_start_line, stmt_span,
    stmt_start_after_operator, stmt_verbatim_span_with_source_map,
    trim_unescaped_trailing_whitespace,
};
use crate::comments::{SourceComment, SourceMap};
use crate::context::RenderContext;
use crate::facts::{FormatterFacts, classify_sequence_contains_heredoc};
use crate::options::{IndentStyle, ResolvedShellFormatOptions};
use crate::raw_syntax::{
    CommandSubstitutionPipelineContinuation, RawLineQuoteState, RawShellText, RenderedHeredocTail,
    command_substitution_context_start, command_substitution_pipeline_stage_continuation,
    line_without_continuation_backslash, normalize_rendered_heredoc_start_spacing,
    redirect_operator_end, rendered_heredoc_tail_start,
    rendered_line_ends_with_structural_pipe_continuation,
    rendered_line_opens_command_substitution_pipeline, rendered_shell_text_has_heredoc_tail,
    skip_double_quoted, skip_single_quoted,
};
use crate::scan::{
    BranchPrefixComment, last_shell_keyword_start, shell_keyword_at, source_between_offsets,
};
use crate::word::{
    normalize_raw_empty_parameter_replacement_delimiters,
    normalize_raw_unquoted_word_continuations, render_arithmetic_expr_to_buf,
    render_escaped_multiline_word_syntax_to_buf, render_heredoc_body_to_buf,
    render_pattern_syntax_to_buf, render_word_syntax_to_buf,
    word_gap_end_before_trailing_continuation, word_is_quoted_command_substitution_only,
    word_is_quoted_formattable_command_substitution_only_with_facts,
};
use branches::{
    branch_prefix_comments_use_disabled_body_indent, if_branch_upper_bound,
    unmodeled_branch_background_operator,
};
use case_layout::{
    case_close_shares_line_with_last_item, case_command_was_inline_in_source,
    case_item_body_can_share_terminator, case_item_body_terminator_was_inline_in_source,
    case_item_body_was_inline_without_terminator, case_item_close_paren_shares_line_with_body,
    case_item_pattern_body_terminator_was_inline_in_source,
    case_item_pattern_close_paren_on_own_line, case_item_pattern_starts_on_case_header,
    case_item_single_body_stmt_can_inline, case_item_started_inline_without_terminator,
    case_prefix_comment_uses_body_indent, comment_looks_like_disabled_case_pattern,
    trim_trailing_pattern_line_continuation,
};
use comments_alignment::{
    inline_comment_code_width, trailing_comment_alignment_column, trailing_comment_padding,
};
use conditions::{
    condition_keyword_on_previous_non_empty_line, condition_stmt_command_end,
    elif_condition_has_explicit_statement_break, if_condition_has_explicit_statement_break,
    if_condition_starts_after_keyword, loop_condition_starts_after_keyword,
    raw_grouped_if_condition, stmt_sequence_renders_with_subshell_open,
};
use gaps::{
    gap_has_blank_line, group_close_offset, stmt_rendered_end_line_after_format,
    stmt_semicolon_terminator_starts_on_continuation_line, trim_trailing_gap_before_offset,
};
use redirects::{
    append_both_redirect_pair_matches_source, redirect_is_attached_process_substitution,
    redirect_list_needs_leading_space, redirect_list_starts_on_continuation_line,
};
use shape::{ExpandedThenFiIfLayout, ThenFiIfLayout};
use substitutions::{
    assignment_source_has_command_substitution, conditional_binary_has_explicit_rhs_break,
    conditional_expr_contains_command_substitution, heredoc_body_contains_command_substitution,
    interstitial_comment_end, pipeline_operator_breaks, word_contains_command_substitution,
};
use writer::{BufferSink, CompareSink, PendingHeredoc, ShellWriter, StreamSink};

pub(crate) fn format_file_streaming(file: &File, context: RenderContext<'_, '_>) -> Result<String> {
    let mut formatter = ShellRenderer::new(context);
    formatter.format_stmt_sequence(&file.body, None)?;

    Ok(formatter.finish_into_string())
}

pub(crate) fn format_file_streaming_matches_source(
    file: &File,
    context: RenderContext<'_, '_>,
) -> Result<bool> {
    let mut formatter = ShellRenderer::new_compare(context);
    formatter.format_stmt_sequence(&file.body, None)?;

    Ok(formatter.finish_matches_source())
}

pub(crate) fn format_stmt_sequence_streaming_to_buf(
    context: RenderContext<'_, '_>,
    statements: &StmtSeq,
    upper_bound: Option<usize>,
    output: &mut String,
) -> Result<()> {
    let mut nested_output = mem::take(output);
    nested_output.clear();

    let mut formatter = ShellRenderer::with_output_buffer(context, nested_output);
    formatter.format_stmt_sequence(statements, upper_bound)?;
    *output = formatter.finish_into_string();
    Ok(())
}

struct ShellRenderer<'source, 'facts, S> {
    source: &'source str,
    options: ResolvedShellFormatOptions,
    facts: &'facts FormatterFacts<'source>,
    scratch: String,
    writer: ShellWriter<S>,
    pipeline_continuation_indent: usize,
    filter_next_group_body_leading_before_open: bool,
}

impl<'source, 'facts> ShellRenderer<'source, 'facts, BufferSink> {
    fn new(context: RenderContext<'source, 'facts>) -> Self {
        Self::with_writer(
            context,
            ShellWriter::new_buffer(context.source, context.options),
        )
    }

    fn with_output_buffer(context: RenderContext<'source, 'facts>, output: String) -> Self {
        Self::with_writer(
            context,
            ShellWriter::with_output_buffer(context.options, output),
        )
    }

    fn finish_into_string(self) -> String {
        self.writer.finish_into_string()
    }
}

impl<'source, 'facts> ShellRenderer<'source, 'facts, CompareSink<'source>> {
    fn new_compare(context: RenderContext<'source, 'facts>) -> Self {
        Self::with_writer(
            context,
            ShellWriter::new_compare(context.source, context.options),
        )
    }

    fn finish_matches_source(self) -> bool {
        self.writer.finish_matches_source()
    }
}

impl<'source, 'facts, S> ShellRenderer<'source, 'facts, S>
where
    S: StreamSink,
{
    fn with_writer(context: RenderContext<'source, 'facts>, writer: ShellWriter<S>) -> Self {
        Self {
            source: context.source,
            options: context.options.clone(),
            facts: context.facts,
            scratch: String::new(),
            writer,
            pipeline_continuation_indent: 1,
            filter_next_group_body_leading_before_open: false,
        }
    }

    fn push_output_str(&mut self, text: &str) {
        self.writer.push_raw_str(text);
    }

    fn source(&self) -> &'source str {
        self.source
    }

    fn options(&self) -> &ResolvedShellFormatOptions {
        &self.options
    }

    fn facts(&self) -> &FormatterFacts<'source> {
        self.facts
    }

    fn source_map(&self) -> &SourceMap<'source> {
        self.render_context().source_map()
    }

    fn render_context(&self) -> RenderContext<'source, '_> {
        RenderContext::new(self.source(), self.options(), self.facts())
    }

    fn line_ending(&self) -> &'static str {
        self.writer.line_ending()
    }

    fn indent_column_for_level(&self, level: usize) -> usize {
        self.writer.indent_column_for_level(level)
    }

    fn indent_level(&self) -> usize {
        self.writer.indent_level()
    }

    fn column(&self) -> usize {
        self.writer.column()
    }

    fn line_indent_column(&self) -> usize {
        self.writer.line_indent_column()
    }

    fn line_start(&self) -> bool {
        self.writer.line_start()
    }

    fn with_indent<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.writer.push_indent(1);
        let result = f(self);
        self.writer.pop_indent(1);
        result
    }

    fn take_scratch_buffer(&mut self) -> String {
        let mut scratch = mem::take(&mut self.scratch);
        scratch.clear();
        scratch
    }

    fn restore_scratch_buffer(&mut self, scratch: String) {
        self.scratch = scratch;
    }

    fn render_word_to_buffer(&self, word: &Word, rendered: &mut String) {
        render_word_syntax_to_buf(word, self.render_context(), rendered);
    }

    fn write_rendered(
        &mut self,
        render: impl FnOnce(&mut String, &'source str, &ResolvedShellFormatOptions),
    ) {
        let mut scratch = self.take_scratch_buffer();
        render(&mut scratch, self.source, &self.options);
        self.write_text(&scratch);
        self.restore_scratch_buffer(scratch);
    }

    fn write_display(&mut self, value: impl std::fmt::Display) {
        self.write_rendered(|scratch, _, _| {
            let _ = write!(scratch, "{value}");
        });
    }

    fn write_indent_units(&mut self, levels: usize) {
        self.writer.write_indent_units(levels);
    }

    fn write_text(&mut self, text: &str) {
        self.writer.write_text(text);
    }

    fn write_verbatim(&mut self, text: &str) {
        self.writer.write_verbatim(text);
    }

    fn write_indent(&mut self) {
        self.writer.write_indent();
    }

    fn write_indent_to_column(&mut self, column: usize) {
        self.writer.write_indent_to_column(column);
    }

    fn write_space(&mut self) {
        self.writer.write_space();
    }

    fn write_spaces(&mut self, count: usize) {
        self.writer.write_spaces(count);
    }

    fn flush_pending_heredocs(&mut self) {
        self.writer.flush_pending_heredocs();
    }

    fn newline(&mut self) {
        self.writer.newline();
    }

    fn line_continuation(&mut self) {
        self.writer.line_continuation();
    }

    fn write_line_breaks(&mut self, count: usize) {
        self.writer.write_line_breaks(count);
    }
}

fn split_first_line(text: &str) -> (&str, &str, bool) {
    text.split_once('\n')
        .map_or((text, "", false), |(line, rest)| (line, rest, true))
}

fn split_first_line_including_newline(text: &str) -> (&str, &str, bool) {
    text.find('\n').map_or((text, "", false), |index| {
        let (line, next) = text.split_at(index + 1);
        (line, next, true)
    })
}

fn last_shell_keyword_span(
    source: &str,
    source_map: &SourceMap<'_>,
    span: Span,
    keyword: &str,
) -> Option<Span> {
    let start = last_shell_keyword_start(source, span, keyword)?;
    Some(source_map.span_for_offsets(start, start + keyword.len()))
}
