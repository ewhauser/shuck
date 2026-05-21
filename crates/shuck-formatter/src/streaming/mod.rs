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

mod comments_alignment;
mod compounds;
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
    render_assignment_head_to_buf, render_assignment_with_facts_to_buf, render_background_operator,
    render_subscript_to_buf, render_var_ref_to_buf, simple_command_uses_synthetic_words,
    slice_span, stmt_attachment_span, stmt_format_span, stmt_render_start_line, stmt_span,
    stmt_start_after_operator, stmt_verbatim_span_with_source_map,
    trim_unescaped_trailing_whitespace,
};
use crate::comments::{SourceComment, SourceMap};
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
    render_escaped_multiline_word_syntax_with_facts_to_buf, render_heredoc_body_to_buf,
    render_pattern_syntax_to_buf, render_word_syntax_with_facts_to_buf,
    word_gap_end_before_trailing_continuation, word_is_quoted_command_substitution_only,
    word_is_quoted_formattable_command_substitution_only_with_facts,
};
use writer::{BufferSink, CompareSink, PendingHeredoc, ShellWriter, StreamSink};

#[derive(Clone, Copy)]
enum SimpleCommandPart<'a> {
    Assignment(&'a Assignment),
    Name,
    Argument(&'a Word),
    Redirect(&'a Redirect),
}

impl SimpleCommandPart<'_> {
    fn start_offset(&self, command: &SimpleCommand) -> usize {
        match self {
            Self::Assignment(assignment) => assignment.span.start.offset,
            Self::Name => command.name.span.start.offset,
            Self::Argument(word) => word.span.start.offset,
            Self::Redirect(redirect) => redirect.span.start.offset,
        }
    }

    fn end_offset(&self, command: &SimpleCommand) -> usize {
        match self {
            Self::Assignment(assignment) => assignment.span.end.offset,
            Self::Name => command.name.span.end.offset,
            Self::Argument(word) => word.span.end.offset,
            Self::Redirect(redirect) => redirect.span.end.offset,
        }
    }

    fn bare_command_gap_end(&self, command: &SimpleCommand, source: &str) -> usize {
        match self {
            Self::Argument(word) => word_gap_end_before_trailing_continuation(word, source),
            _ => self.end_offset(command),
        }
    }
}

fn move_interspersed_redirects_after_arguments<'a>(parts: &mut Vec<SimpleCommandPart<'a>>) {
    let mut saw_argument = false;
    let mut deferred_redirects = Vec::new();
    let mut reordered = Vec::with_capacity(parts.len());

    for part in parts.drain(..) {
        match part {
            SimpleCommandPart::Argument(_) => {
                saw_argument = true;
                reordered.push(part);
            }
            SimpleCommandPart::Redirect(_) if saw_argument => deferred_redirects.push(part),
            _ => reordered.push(part),
        }
    }

    reordered.extend(deferred_redirects);
    *parts = reordered;
}

pub(crate) fn format_file_streaming_with_facts(
    source: &str,
    file: &File,
    options: &ResolvedShellFormatOptions,
    facts: &FormatterFacts<'_>,
) -> Result<String> {
    let mut formatter = ShellRenderer::new(source, options, facts);
    formatter.format_stmt_sequence(&file.body, None)?;

    Ok(formatter.finish_into_string())
}

pub(crate) fn format_file_streaming_matches_source_with_facts(
    source: &str,
    file: &File,
    options: &ResolvedShellFormatOptions,
    facts: &FormatterFacts<'_>,
) -> Result<bool> {
    let mut formatter = ShellRenderer::new_compare(source, options, facts);
    formatter.format_stmt_sequence(&file.body, None)?;

    Ok(formatter.finish_matches_source())
}

pub(crate) fn format_stmt_sequence_streaming_to_buf(
    source: &str,
    statements: &StmtSeq,
    options: &ResolvedShellFormatOptions,
    facts: &FormatterFacts<'_>,
    upper_bound: Option<usize>,
    output: &mut String,
) -> Result<()> {
    let mut nested_output = mem::take(output);
    nested_output.clear();

    let mut formatter = ShellRenderer::with_output_buffer(source, options, facts, nested_output);
    formatter.format_stmt_sequence(statements, upper_bound)?;
    *output = formatter.finish_into_string();
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct BinaryListItem<'a> {
    operator: BinaryOp,
    operator_span: Span,
    stmt: &'a Stmt,
}

#[derive(Debug, Clone, Copy)]
enum MultilineCompoundAssignmentPlacement {
    Inline,
    Standalone,
}

#[derive(Debug, Clone, Copy)]
enum HeredocTailTextMode {
    Rendered,
    Assignment,
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
    fn new(
        source: &'source str,
        options: &ResolvedShellFormatOptions,
        facts: &'facts FormatterFacts<'source>,
    ) -> Self {
        Self::with_writer(
            source,
            options,
            facts,
            ShellWriter::new_buffer(source, options),
        )
    }

    fn with_output_buffer(
        source: &'source str,
        options: &ResolvedShellFormatOptions,
        facts: &'facts FormatterFacts<'source>,
        output: String,
    ) -> Self {
        Self::with_writer(
            source,
            options,
            facts,
            ShellWriter::with_output_buffer(options, output),
        )
    }

    fn finish_into_string(self) -> String {
        self.writer.finish_into_string()
    }
}

impl<'source, 'facts> ShellRenderer<'source, 'facts, CompareSink<'source>> {
    fn new_compare(
        source: &'source str,
        options: &ResolvedShellFormatOptions,
        facts: &'facts FormatterFacts<'source>,
    ) -> Self {
        Self::with_writer(
            source,
            options,
            facts,
            ShellWriter::new_compare(source, options),
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
    fn with_writer(
        source: &'source str,
        options: &ResolvedShellFormatOptions,
        facts: &'facts FormatterFacts<'source>,
        writer: ShellWriter<S>,
    ) -> Self {
        Self {
            source,
            options: options.clone(),
            facts,
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
        self.facts.source_map()
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

    fn render_word_with_facts_to_buffer(&self, word: &Word, rendered: &mut String) {
        let source_map = self.source_map().clone();
        let facts = self.facts();
        render_word_syntax_with_facts_to_buf(
            word,
            self.source(),
            self.options(),
            &source_map,
            facts,
            rendered,
        );
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

fn assignment_contains_command_heredoc(assignment: &Assignment) -> bool {
    match &assignment.value {
        AssignmentValue::Scalar(word) => word_contains_command_heredoc(word),
        AssignmentValue::Compound(array) => array
            .elements
            .iter()
            .any(|element| word_contains_command_heredoc(array_elem_parts(element).1)),
    }
}

fn compound_assignment_is_single_case_command_substitution(assignment: &Assignment) -> bool {
    let AssignmentValue::Compound(array) = &assignment.value else {
        return false;
    };
    let [ArrayElem::Sequential(word)] = array.elements.as_slice() else {
        return false;
    };
    let [part] = word.parts.as_slice() else {
        return false;
    };
    let WordPart::CommandSubstitution { body, .. } = &part.kind else {
        return false;
    };
    matches!(
        body.as_slice(),
        [stmt]
            if !stmt.negated
                && stmt.redirects.is_empty()
                && matches!(&stmt.command, Command::Compound(CompoundCommand::Case(_)))
    )
}

fn word_contains_command_heredoc(word: &Word) -> bool {
    word.parts
        .iter()
        .any(|part| word_part_contains_command_heredoc(&part.kind))
}

fn heredoc_body_contains_command_substitution(body: &HeredocBody) -> bool {
    body.parts
        .iter()
        .any(|part| matches!(part.kind, HeredocBodyPart::CommandSubstitution { .. }))
}

fn word_part_contains_command_heredoc(part: &WordPart) -> bool {
    match part {
        WordPart::CommandSubstitution { body, .. } | WordPart::ProcessSubstitution { body, .. } => {
            classify_sequence_contains_heredoc(body)
        }
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter()
            .any(|part| word_part_contains_command_heredoc(&part.kind)),
        _ => false,
    }
}

fn raw_redirect_source_slice<'a>(redirect: &Redirect, source: &'a str) -> Option<&'a str> {
    let span = redirect.span;
    (span.start.offset < span.end.offset && span.end.offset <= source.len())
        .then(|| span.slice(source))
}

fn should_preserve_raw_redirect(raw: &str) -> bool {
    raw.contains(">&$")
        || raw.contains("<&$")
        || raw.contains(">&-")
        || raw.contains("<&-")
        || raw.contains(">&/")
        || raw.contains("<&/")
}

fn append_both_redirect_pair_matches_source(
    redirect: &Redirect,
    next: &Redirect,
    source: &str,
) -> bool {
    if !matches!(redirect.kind, RedirectKind::Append)
        || redirect.fd.is_some()
        || redirect.fd_var.is_some()
    {
        return false;
    }
    if !matches!(next.kind, RedirectKind::DupOutput)
        || next.fd != Some(2)
        || next
            .word_target()
            .and_then(|word| word.try_static_text(source))
            .is_none_or(|target| target != "1")
    {
        return false;
    }

    let Some(raw) = raw_redirect_source_slice(redirect, source) else {
        return false;
    };
    if raw.starts_with("&>>") {
        return true;
    }
    if raw.starts_with(">>") {
        let Some(operator_start) = redirect.span.start.offset.checked_sub(1) else {
            return false;
        };
        return source
            .as_bytes()
            .get(operator_start)
            .is_some_and(|byte| *byte == b'&');
    }
    false
}

fn redirect_target_starts_on_continuation_line(
    redirect: &Redirect,
    facts: &FormatterFacts<'_>,
) -> bool {
    let target_start = redirect
        .word_target()
        .map(|word| word.span.start.offset)
        .or_else(|| {
            redirect
                .heredoc()
                .map(|heredoc| heredoc.delimiter.span.start.offset)
        });
    let Some(target_start) = target_start else {
        return false;
    };
    facts.has_continuation_line_start_between(redirect.span.start.offset, target_start)
}

fn should_render_explicit_fd(fd: i32, kind: RedirectKind) -> bool {
    match kind {
        RedirectKind::Output
        | RedirectKind::Clobber
        | RedirectKind::Append
        | RedirectKind::DupOutput
        | RedirectKind::OutputBoth => fd != 1,
        RedirectKind::Input
        | RedirectKind::ReadWrite
        | RedirectKind::HereDoc
        | RedirectKind::HereDocStrip
        | RedirectKind::HereString
        | RedirectKind::DupInput => fd != 0,
    }
}

fn redirect_source_has_explicit_fd(redirect: &Redirect, source: &str, fd: i32) -> bool {
    let Some(raw) = raw_redirect_source_slice(redirect, source) else {
        return false;
    };
    let rendered_fd = fd.to_string();
    raw.trim_start().starts_with(&rendered_fd)
}

fn redirect_has_adjacent_numeric_fd_prefix(
    previous_part: Option<SimpleCommandPart<'_>>,
    redirect: &Redirect,
    command: &SimpleCommand,
    source: &str,
) -> bool {
    if matches!(redirect.kind, RedirectKind::OutputBoth) {
        return false;
    }
    let Some(SimpleCommandPart::Argument(word)) = previous_part else {
        return false;
    };
    if word.span.end.offset != redirect.span.start.offset {
        return false;
    }
    let Some(raw) = source.get(word.span.start.offset..word.span.end.offset) else {
        return false;
    };
    raw.chars().all(|ch| ch.is_ascii_digit())
        && word.span.start.offset > command.name.span.end.offset
}

fn needs_space_before_target(kind: RedirectKind, target: &str, space_redirects: bool) -> bool {
    if target.is_empty() {
        return false;
    }
    if space_redirects && !matches!(kind, RedirectKind::DupOutput | RedirectKind::DupInput) {
        return true;
    }
    !matches!(kind, RedirectKind::DupOutput | RedirectKind::DupInput)
        && target
            .as_bytes()
            .first()
            .is_some_and(|byte| matches!(byte, b'<' | b'>' | b'&'))
}

fn normalized_redirect_target(kind: RedirectKind, target: &str) -> &str {
    if matches!(
        kind,
        RedirectKind::Output
            | RedirectKind::Clobber
            | RedirectKind::Append
            | RedirectKind::Input
            | RedirectKind::ReadWrite
            | RedirectKind::HereDoc
            | RedirectKind::HereDocStrip
            | RedirectKind::HereString
            | RedirectKind::OutputBoth
    ) {
        target.trim_start_matches([' ', '\t', '\r'])
    } else {
        target
    }
}

fn here_string_target_is_multiline_literal(target: &str) -> bool {
    let target = target.strip_prefix('$').unwrap_or(target);
    target.starts_with("\"\n")
        || target.starts_with("\"\\\n")
        || target.starts_with("'\n")
        || target.starts_with("$'\n")
        || target.starts_with("\"\r\n")
        || target.starts_with("\"\\\r\n")
        || target.starts_with("'\r\n")
        || target.starts_with("$'\r\n")
}

fn redirect_list_needs_leading_space(
    command_span: Span,
    redirects: &[Redirect],
    source: &str,
) -> bool {
    redirects.first().is_none_or(|redirect| {
        !redirect_is_attached_process_substitution(command_span, redirect, source)
    })
}

fn redirect_list_starts_on_continuation_line(
    command_span: Span,
    redirects: &[Redirect],
    facts: &FormatterFacts<'_>,
) -> bool {
    let Some(redirect) = redirects.first() else {
        return false;
    };
    if command_span == Span::new() || redirect.span.start.offset <= command_span.end.offset {
        return false;
    }
    facts.has_continuation_line_start_between(command_span.end.offset, redirect.span.start.offset)
}

fn redirect_is_attached_process_substitution(
    _command_span: Span,
    redirect: &Redirect,
    source: &str,
) -> bool {
    let start = redirect.span.start.offset;
    let bytes = source.as_bytes();
    let attached_after_equals = start > 0 && bytes.get(start - 1).is_some_and(|byte| *byte == b'=')
        || start > 1
            && bytes
                .get(start - 1)
                .is_some_and(|byte| matches!(*byte, b'<' | b'>'))
            && bytes.get(start - 2).is_some_and(|byte| *byte == b'=');
    attached_after_equals
        && raw_redirect_source_slice(redirect, source).is_some_and(|raw| {
            raw.starts_with("<(") || raw.starts_with(">(") || raw.starts_with('(')
        })
}

fn normalize_escaped_multiline_word_command_substitution_indent(
    rendered: &str,
    options: &ResolvedShellFormatOptions,
) -> Option<String> {
    let normalized = rendered.strip_prefix('$').unwrap_or(rendered);
    if !normalized.starts_with("\"\\\n") && !normalized.starts_with("\"\\\r\n") {
        return None;
    }

    let indent = options.indent_prefix(1);
    let mut output = String::with_capacity(rendered.len() + indent.len() * 4);
    let mut changed = false;
    let mut command_substitution_depth = 0usize;

    for (index, line) in rendered.split('\n').enumerate() {
        if index > 0 {
            output.push('\n');
        }

        let trimmed = line.trim_start_matches([' ', '\t']);
        if command_substitution_depth > 0 {
            if !line.is_empty() {
                output.push_str(&indent);
                changed = true;
            }
            output.push_str(line);
            if trimmed.starts_with(')') {
                command_substitution_depth = command_substitution_depth.saturating_sub(1);
            }
            if trimmed.ends_with("$(") {
                command_substitution_depth += 1;
            }
            continue;
        }

        output.push_str(line);
        if trimmed.ends_with("$(") {
            command_substitution_depth = 1;
        }
    }

    changed.then_some(output)
}

fn normalize_rendered_leading_list_operator_continuations(rendered: &str) -> Option<String> {
    let mut output = Vec::<String>::new();
    let mut changed = false;

    for line in rendered.split('\n') {
        let mut current = line.to_string();
        if let Some((operator, rest)) = leading_list_operator_line_parts(line)
            && let Some(previous) = output.last_mut()
            && let Some(prefix_len) = line_without_continuation_backslash(previous).map(str::len)
        {
            previous.truncate(prefix_len);
            previous.push(' ');
            previous.push_str(operator);
            current.clear();
            current.push_str(rest);
            changed = true;
        }
        output.push(current);
    }

    changed.then(|| output.join("\n"))
}

fn leading_list_operator_line_parts(line: &str) -> Option<(&'static str, &str)> {
    let trimmed = line.trim_start_matches([' ', '\t', '\r']);
    let (operator, rest) = if let Some(rest) = trimmed.strip_prefix("||") {
        ("||", rest)
    } else if let Some(rest) = trimmed.strip_prefix("&&") {
        ("&&", rest)
    } else if let Some(rest) = trimmed.strip_prefix("|&") {
        ("|&", rest)
    } else if let Some(rest) = trimmed.strip_prefix('|') {
        if trimmed.starts_with("|)") {
            return None;
        }
        ("|", rest)
    } else {
        return None;
    };

    Some((operator, rest.trim_start_matches([' ', '\t', '\r'])))
}

fn normalize_scalar_assignment_unquoted_continuations(
    assignment: &Assignment,
    source: &str,
    facts: &FormatterFacts,
) -> Option<String> {
    if assignment_source_has_command_substitution(assignment, source) {
        return None;
    }
    let AssignmentValue::Scalar(_) = &assignment.value else {
        return None;
    };
    if !facts.has_raw_continuation_backslash_between(
        assignment.span.start.offset,
        assignment.span.end.offset,
    ) {
        return None;
    }

    let raw = assignment.span.slice(source);
    let mut head = String::new();
    render_assignment_head_to_buf(assignment, source, &mut head);
    let raw_value = raw.strip_prefix(&head)?;
    let normalized_value = normalize_raw_unquoted_word_continuations(raw_value)?;
    let mut normalized = head;
    normalized.push_str(&normalized_value);
    Some(normalized)
}

fn arithmetic_command_is_followed_by_inline_branch_keyword(span: Span, source: &str) -> bool {
    let Some(after) = source.get(span.end.offset.min(source.len())..) else {
        return false;
    };
    let trimmed = after.trim_start_matches([' ', '\t', '\r']);
    let after_separator = trimmed
        .strip_prefix(';')
        .map(|rest| rest.trim_start_matches([' ', '\t', '\r']))
        .unwrap_or(trimmed);

    shell_keyword_prefix(after_separator, "then") || shell_keyword_prefix(after_separator, "do")
}

fn shell_keyword_prefix(text: &str, keyword: &str) -> bool {
    let Some(rest) = text.strip_prefix(keyword) else {
        return false;
    };
    rest.chars()
        .next()
        .is_none_or(|ch| matches!(ch, ' ' | '\t' | '\r' | '\n' | ';' | '&' | '|'))
}

fn trim_final_line_ending(text: &mut String) {
    if text.ends_with("\r\n") {
        text.truncate(text.len().saturating_sub(2));
    } else if text.ends_with('\n') {
        text.truncate(text.len().saturating_sub(1));
    }
}

fn assignment_has_quoted_backslash_continuation_literal(
    assignment: &Assignment,
    source: &str,
) -> bool {
    let AssignmentValue::Scalar(_) = &assignment.value else {
        return false;
    };
    let raw = assignment.span.slice(source);
    raw.contains("\\\n")
        && RawShellText::new(raw).quoted_backslash_continuation()
        && !raw.contains("$(")
        && !raw.contains('`')
        && !raw.contains("<(")
        && !raw.contains(">(")
}

fn assignment_source_has_command_substitution(assignment: &Assignment, source: &str) -> bool {
    let raw = assignment.span.slice(source);
    raw.contains("$(") || raw.contains('`') || raw.contains("<(") || raw.contains(">(")
}

fn assignment_source_has_leading_pipe_continuation(assignment: &Assignment, source: &str) -> bool {
    let raw = assignment.span.slice(source);
    let mut rest = raw;
    while let Some(index) = rest.find("\\\n") {
        let after_break = &rest[index + 2..];
        let trimmed = after_break.trim_start_matches([' ', '\t', '\r']);
        if trimmed.starts_with('|') && !trimmed.starts_with("||") {
            return true;
        }
        rest = after_break;
    }
    false
}

fn assignment_value_is_quoted_formattable_command_substitution_only(
    assignment: &Assignment,
    facts: &FormatterFacts<'_>,
) -> bool {
    match &assignment.value {
        AssignmentValue::Scalar(word) => {
            word_is_quoted_formattable_command_substitution_only_with_facts(word, facts)
        }
        AssignmentValue::Compound(_) => false,
    }
}

fn assignment_value_is_quoted_command_substitution_only(assignment: &Assignment) -> bool {
    match &assignment.value {
        AssignmentValue::Scalar(word) => word_is_quoted_command_substitution_only(word),
        AssignmentValue::Compound(_) => false,
    }
}

fn stmt_semicolon_terminator_starts_on_continuation_line(
    stmt: &Stmt,
    source_map: &SourceMap<'_>,
) -> bool {
    let Some(terminator_span) = stmt.terminator_span else {
        return false;
    };
    let render_end = stmt
        .redirects
        .last()
        .map(|redirect| redirect.span.end.offset)
        .unwrap_or_else(|| command_format_span(&stmt.command).end.offset);
    source_map.contains_newline_between(render_end, terminator_span.start.offset)
}

fn stmt_rendered_end_line_after_format(
    stmt: &Stmt,
    source: &str,
    source_map: &SourceMap<'_>,
    fallback: usize,
) -> usize {
    if matches!(stmt.terminator, Some(StmtTerminator::Semicolon))
        && stmt_semicolon_terminator_starts_on_continuation_line(stmt, source_map)
        && let Some(terminator_span) = stmt.terminator_span
    {
        return terminator_span.start.line;
    }
    match &stmt.command {
        Command::Binary(command) => {
            return stmt_rendered_end_line_after_format(
                command.right.as_ref(),
                source,
                source_map,
                fallback,
            );
        }
        _ if stmt.redirects.is_empty() && stmt.terminator.is_none() => {
            if let Some((commands, open)) = command_group_commands(&stmt.command)
                && let Some(span) = group_attachment_span(
                    commands.as_slice(),
                    source_map,
                    open,
                    matching_group_close(open),
                )
            {
                let close = matching_group_close(open);
                let close_offset = group_close_offset(
                    source,
                    span,
                    Some(stmt_span(stmt).end.offset),
                    close,
                    close.len_utf8(),
                );
                return source_map.line_number_for_offset(close_offset);
            }
        }
        _ => {}
    }
    fallback
}

fn if_condition_starts_after_keyword(
    command: &IfCommand,
    then_span: Span,
    source: &str,
    source_map: &SourceMap<'_>,
    options: &ResolvedShellFormatOptions,
    facts: &FormatterFacts,
) -> bool {
    if raw_if_condition_starts_with_negation_continuation(command, then_span, source, facts) {
        return false;
    }
    command.condition.first().is_some_and(|stmt| {
        stmt_render_start_line(stmt, source, source_map, options) > command.span.start.line
    })
}

fn if_condition_has_explicit_statement_break(
    command: &IfCommand,
    then_span: Span,
    source: &str,
    source_map: &SourceMap<'_>,
    facts: &FormatterFacts,
) -> bool {
    if raw_if_condition_starts_with_negation_continuation(command, then_span, source, facts) {
        return false;
    }
    condition_sequence_has_explicit_statement_break(
        &command.condition,
        then_span.start.offset,
        source,
        source_map,
    )
}

fn raw_if_condition_starts_with_negation_continuation(
    command: &IfCommand,
    then_span: Span,
    source: &str,
    facts: &FormatterFacts,
) -> bool {
    let condition_start = command.span.start.offset.saturating_add("if".len());
    let condition_end = then_span.start.offset.min(source.len());
    let Some(raw) = source.get(condition_start..condition_end) else {
        return false;
    };
    let raw = raw.trim_start_matches([' ', '\t', '\r']);
    let Some(after_negation) = raw.strip_prefix('!') else {
        return false;
    };
    let after_negation = after_negation.trim_start_matches([' ', '\t', '\r']);
    let continuation_offset = condition_end - after_negation.len();
    facts.has_raw_continuation_backslash_between(
        continuation_offset,
        continuation_offset.saturating_add(1),
    )
}

fn condition_sequence_has_explicit_statement_break(
    condition: &StmtSeq,
    upper_bound: usize,
    source: &str,
    source_map: &SourceMap<'_>,
) -> bool {
    if condition.len() == 1 {
        let Some(stmt) = condition.first() else {
            return false;
        };
        if !matches!(stmt.command, Command::Simple(_)) {
            return false;
        }
        let start = stmt_span(stmt).start.offset;
        let command_end = condition_stmt_command_end(stmt).min(upper_bound);
        return source
            .get(start..command_end)
            .is_some_and(has_unescaped_line_break);
    }

    condition.as_slice().windows(2).any(|pair| {
        let previous_start = stmt_span(&pair[0]).start.offset;
        let next_start = stmt_span(&pair[1]).start.offset;
        source_map.contains_newline_between(previous_start, next_start)
    })
}

fn condition_stmt_command_end(stmt: &Stmt) -> usize {
    let mut end = command_format_span(&stmt.command).end.offset;
    if end == 0 {
        end = stmt_span(stmt).end.offset;
    }
    for redirect in &stmt.redirects {
        end = end.max(redirect.span.end.offset);
    }
    end
}

fn elif_condition_has_explicit_statement_break(
    condition: &StmtSeq,
    body: &StmtSeq,
    source: &str,
    source_map: &SourceMap<'_>,
) -> bool {
    let upper_bound =
        branch_open_keyword_start(body, source, "then").unwrap_or(body.span.start.offset);
    condition_sequence_has_explicit_statement_break(condition, upper_bound, source, source_map)
}

fn has_unescaped_line_break(text: &str) -> bool {
    let mut cursor = 0usize;
    let upper = text.len();
    while cursor < upper {
        let Some(ch) = text[cursor..].chars().next() else {
            break;
        };
        match ch {
            '\'' => {
                cursor = skip_single_quoted(text, cursor + ch.len_utf8(), upper);
                continue;
            }
            '"' => {
                cursor = skip_double_quoted(text, cursor + ch.len_utf8(), upper);
                continue;
            }
            '\n' => {
                let before = text[..cursor].trim_end_matches([' ', '\t', '\r']);
                if !before.ends_with('\\') {
                    return true;
                }
            }
            _ => {}
        }
        cursor += ch.len_utf8();
    }
    false
}

fn loop_condition_starts_after_keyword(condition: &StmtSeq, span: Span) -> bool {
    condition
        .first()
        .is_some_and(|stmt| stmt_span(stmt).start.line > span.start.line)
}

fn condition_keyword_on_previous_non_empty_line(
    condition: &StmtSeq,
    source: &str,
    source_map: &SourceMap<'_>,
    keyword: &str,
) -> bool {
    let Some(first) = condition.first() else {
        return false;
    };
    let Some((mut line_start, _)) =
        source_map.line_bounds_for_offset(stmt_span(first).start.offset)
    else {
        return false;
    };

    while let Some((start, end)) = source_map.previous_line_bounds(line_start) {
        let Some(line) = source.get(start..end) else {
            return false;
        };
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            return trimmed == keyword;
        }
        line_start = start;
    }

    false
}

fn raw_grouped_if_condition(
    command: &IfCommand,
    then_span: Span,
    source: &str,
    source_map: &SourceMap<'_>,
    options: &ResolvedShellFormatOptions,
    facts: &FormatterFacts,
) -> Option<String> {
    if !if_condition_starts_after_keyword(command, then_span, source, source_map, options, facts) {
        return None;
    }
    let start = command.span.start.offset.checked_add("if".len())?;
    let end = then_span.start.offset;
    if start >= end || end > source.len() {
        return None;
    }
    let raw = source.get(start..end)?;
    if !(raw.trim_start().starts_with('{') && raw.contains('}') && raw.contains('\n')) {
        return None;
    }
    let outer_indent = source_map
        .line_indent_before_offset(command.span.start.offset)
        .unwrap_or("");
    Some(strip_outer_indent_after_first_line(raw, outer_indent))
}

fn strip_outer_indent_after_first_line(raw: &str, outer_indent: &str) -> String {
    if outer_indent.is_empty() {
        return raw.to_string();
    }

    let mut normalized = String::with_capacity(raw.len());
    let mut lines = raw.split('\n');
    if let Some(first) = lines.next() {
        normalized.push_str(first);
    }
    for line in lines {
        normalized.push('\n');
        normalized.push_str(line.strip_prefix(outer_indent).unwrap_or(line));
    }
    normalized
}

fn stmt_sequence_renders_with_subshell_open(commands: &StmtSeq) -> bool {
    commands
        .first()
        .is_some_and(stmt_renders_with_subshell_open)
}

fn stmt_renders_with_subshell_open(stmt: &Stmt) -> bool {
    if stmt.negated {
        return false;
    }
    let command_start = command_format_span(&stmt.command).start.offset;
    if stmt
        .redirects
        .iter()
        .any(|redirect| redirect.span.start.offset < command_start)
    {
        return false;
    }
    match &stmt.command {
        Command::Binary(command) => stmt_renders_with_subshell_open(&command.left),
        Command::Compound(CompoundCommand::Subshell(_)) => true,
        _ => false,
    }
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

fn gap_has_blank_line(source: &str, start: usize, end: usize) -> bool {
    source_between_offsets(source, start, end)
        .is_some_and(|gap| gap.bytes().filter(|byte| *byte == b'\n').count() >= 2)
}

fn case_command_was_inline_in_source(command: &CaseCommand, source: &str) -> bool {
    command.span.slice(source).lines().nth(1).is_none()
}

fn case_item_body_terminator_was_inline_in_source(item: &CaseItem) -> bool {
    let [stmt] = item.body.as_slice() else {
        return false;
    };
    item.terminator_span
        .is_some_and(|span| span.start.line == stmt_format_span(stmt).end.line)
}

fn case_item_pattern_body_terminator_was_inline_in_source(item: &CaseItem, source: &str) -> bool {
    let Some(pattern) = item.patterns.last() else {
        return false;
    };
    let [stmt] = item.body.as_slice() else {
        return false;
    };
    let Some(terminator_span) = item.terminator_span else {
        return false;
    };
    let pattern_end = pattern.span.end.offset.min(source.len());
    let stmt_start = stmt_span(stmt).start.offset.min(source.len());
    let stmt_end = stmt_format_span(stmt).end.offset.min(source.len());
    let terminator_start = terminator_span.start.offset.min(source.len());
    let pattern_and_body_share_line = pattern.span.end.line == stmt_span(stmt).start.line
        || source
            .get(pattern_end..stmt_start)
            .is_some_and(|gap| !gap.contains('\n') && !gap.contains('\r'));
    let body_and_terminator_share_line = terminator_span.start.line
        == stmt_format_span(stmt).end.line
        || source
            .get(stmt_end..terminator_start)
            .is_some_and(|gap| !gap.contains('\n') && !gap.contains('\r'))
        || case_item_source_line_has_terminator_after_body(item, stmt, source);
    pattern_and_body_share_line && body_and_terminator_share_line
}

fn case_item_source_line_has_terminator_after_body(
    item: &CaseItem,
    stmt: &Stmt,
    source: &str,
) -> bool {
    let stmt_end = stmt_format_span(stmt).end.offset.min(source.len());
    let line_end = source[stmt_end..]
        .find(['\n', '\r'])
        .map_or(source.len(), |offset| stmt_end + offset);
    source
        .get(stmt_end..line_end)
        .is_some_and(|tail| tail.contains(case_terminator(item.terminator)))
}

fn case_item_body_can_share_terminator(item: &CaseItem) -> bool {
    let [stmt] = item.body.as_slice() else {
        return false;
    };
    matches!(
        stmt.command,
        Command::Simple(_) | Command::Builtin(_) | Command::Decl(_)
    ) && stmt.redirects.is_empty()
        && stmt.terminator.is_none()
}

fn case_item_single_body_stmt_can_inline(
    item: &CaseItem,
    source: &str,
    source_map: &SourceMap<'_>,
    pattern_body_terminator_was_inline: bool,
) -> bool {
    let [stmt] = item.body.as_slice() else {
        return false;
    };
    if let Command::Compound(CompoundCommand::If(command)) = &stmt.command {
        return pattern_body_terminator_was_inline
            && case_item_close_paren_shares_line_with_body(item, source, source_map)
            && case_item_if_close_shares_terminator(command, item, source, source_map);
    }
    if let Command::Compound(CompoundCommand::Case(command)) = &stmt.command {
        return pattern_body_terminator_was_inline
            && case_item_case_close_shares_terminator(command, item, source, source_map);
    }
    true
}

fn case_item_if_close_shares_terminator(
    command: &IfCommand,
    item: &CaseItem,
    source: &str,
    source_map: &SourceMap<'_>,
) -> bool {
    let Some(terminator_span) = item.terminator_span else {
        return false;
    };
    let fi_span = command_if_close_span(command, source, source_map);
    let fi_end = fi_span.end.offset.min(source.len());
    let terminator_start = terminator_span.start.offset.min(source.len());
    source
        .get(fi_end..terminator_start)
        .is_some_and(|gap| !gap.contains('\n') && !gap.contains('\r'))
}

fn case_item_case_close_shares_terminator(
    command: &CaseCommand,
    item: &CaseItem,
    source: &str,
    source_map: &SourceMap<'_>,
) -> bool {
    let Some(terminator_span) = item.terminator_span else {
        return false;
    };
    let Some(esac_span) = last_shell_keyword_span(source, source_map, command.span, "esac") else {
        return false;
    };
    let esac_end = esac_span.end.offset.min(source.len());
    let terminator_start = terminator_span.start.offset.min(source.len());
    source
        .get(esac_end..terminator_start)
        .is_some_and(|gap| !gap.contains('\n') && !gap.contains('\r'))
}

fn case_item_body_was_inline_without_terminator(item: &CaseItem) -> bool {
    if item.terminator_span.is_some() || !case_item_body_can_share_terminator(item) {
        return false;
    }
    let Some(pattern) = item.patterns.last() else {
        return false;
    };
    let Some(stmt) = item.body.first() else {
        return false;
    };
    pattern.span.end.line == stmt_span(stmt).start.line
}

fn case_close_shares_line_with_last_item(
    command: &CaseCommand,
    esac_span: Option<Span>,
    source: &str,
) -> bool {
    let Some(esac_span) = esac_span else {
        return false;
    };
    let Some(last_item) = command.cases.last() else {
        return false;
    };
    let Some(terminator_span) = last_item.terminator_span else {
        return false;
    };
    let terminator_end = terminator_span.end.offset.min(source.len());
    let esac_start = esac_span.start.offset.min(source.len());
    source
        .get(terminator_end..esac_start)
        .is_some_and(|gap| !gap.contains('\n') && !gap.contains('\r'))
}

fn case_item_started_inline_without_terminator(item: &CaseItem) -> bool {
    if item.terminator_span.is_some() {
        return false;
    }
    let Some(pattern) = item.patterns.last() else {
        return false;
    };
    let [stmt] = item.body.as_slice() else {
        return false;
    };
    pattern.span.end.line == stmt_span(stmt).start.line
}

fn case_item_pattern_starts_on_case_header(command: &CaseCommand, item: &CaseItem) -> bool {
    item.patterns
        .first()
        .is_some_and(|pattern| pattern.span.start.line == command.span.start.line)
}

fn case_item_pattern_close_paren_on_own_line(
    item: &CaseItem,
    source: &str,
    source_map: &SourceMap<'_>,
) -> bool {
    let Some(first_pattern) = item.patterns.first() else {
        return false;
    };
    let end = item
        .body
        .first()
        .map(stmt_span)
        .map(|span| span.start.offset)
        .or_else(|| item.terminator_span.map(|span| span.start.offset))
        .unwrap_or(item.body.span.start.offset);
    let Some(slice) = source.get(first_pattern.span.start.offset..end) else {
        return false;
    };
    let Some(close_offset) = slice.rfind(')') else {
        return false;
    };
    let close_offset = first_pattern.span.start.offset + close_offset;
    let Some((line_start, _)) = source_map.line_bounds_for_offset(close_offset) else {
        return false;
    };
    source
        .get(line_start..close_offset)
        .unwrap_or("")
        .trim_matches([' ', '\t', '\r'])
        .is_empty()
}

fn case_item_close_paren_shares_line_with_body(
    item: &CaseItem,
    source: &str,
    source_map: &SourceMap<'_>,
) -> bool {
    let Some(first_pattern) = item.patterns.first() else {
        return false;
    };
    let Some(first_stmt) = item.body.first() else {
        return false;
    };
    let stmt_start = stmt_span(first_stmt).start.offset.min(source.len());
    let Some(slice) = source.get(first_pattern.span.start.offset..stmt_start) else {
        return false;
    };
    let Some(close_offset) = slice.rfind(')') else {
        return false;
    };
    let close_offset = first_pattern.span.start.offset + close_offset;
    !source_map.contains_newline_between(close_offset + 1, stmt_start)
}

fn trim_trailing_pattern_line_continuation(rendered: &mut String) {
    let trimmed = rendered.trim_end_matches([' ', '\t', '\r']);
    if let Some(stripped) = trimmed.strip_suffix("\\\n") {
        rendered.truncate(stripped.len());
        return;
    }
    let Some(stripped) = trimmed.strip_suffix('\\') else {
        return;
    };
    rendered.truncate(stripped.len());
}

fn brace_group_last_stmt_allows_done_without_semicolon(commands: &StmtSeq) -> bool {
    let Some(last) = commands.last() else {
        return false;
    };
    command_allows_done_without_semicolon(&last.command)
}

fn command_allows_done_without_semicolon(command: &Command) -> bool {
    match command {
        Command::Compound(command) => compound_allows_done_without_semicolon(command),
        Command::Binary(binary) => command_allows_done_without_semicolon(&binary.right.command),
        _ => false,
    }
}

fn compound_allows_done_without_semicolon(command: &CompoundCommand) -> bool {
    match command {
        CompoundCommand::Case(_) => true,
        CompoundCommand::BraceGroup(commands)
        | CompoundCommand::For(ForCommand { body: commands, .. })
        | CompoundCommand::Repeat(RepeatCommand { body: commands, .. })
        | CompoundCommand::Foreach(ForeachCommand { body: commands, .. })
        | CompoundCommand::While(WhileCommand { body: commands, .. })
        | CompoundCommand::Until(UntilCommand { body: commands, .. })
        | CompoundCommand::Select(SelectCommand { body: commands, .. }) => {
            brace_group_last_stmt_allows_done_without_semicolon(commands)
        }
        CompoundCommand::ArithmeticFor(command) => {
            brace_group_last_stmt_allows_done_without_semicolon(&command.body)
        }
        _ => false,
    }
}

fn group_close_offset(
    source: &str,
    span: Span,
    upper_bound: Option<usize>,
    close_char: char,
    close_len: usize,
) -> usize {
    let fallback = span.end.offset.saturating_sub(close_len);
    let search_end = upper_bound
        .map(|offset| offset.saturating_add(close_len))
        .unwrap_or(span.end.offset)
        .min(source.len())
        .max(span.start.offset);
    source
        .get(span.start.offset..search_end)
        .and_then(|text| text.rfind(close_char))
        .map_or(fallback, |offset| span.start.offset + offset)
}

fn trailing_comment_padding(
    source: &str,
    source_map: &SourceMap<'_>,
    comment: &SourceComment<'_>,
    current_code_column: usize,
    current_indent_column: usize,
) -> usize {
    let Some(target_column) =
        trailing_comment_alignment_column(source, source_map, comment, current_indent_column)
    else {
        return 1;
    };
    let indent_adjust = trailing_comment_tab_indent_adjust(source, source_map, comment);
    target_column
        .saturating_sub(current_code_column.saturating_add(indent_adjust))
        .max(1)
}

fn close_suffix_comment_padding(
    source: &str,
    source_map: &SourceMap<'_>,
    comment: &SourceComment<'_>,
    current_code_column: usize,
    current_indent_column: usize,
) -> usize {
    if let Some(padding) = aligned_close_suffix_comment_padding(
        source,
        source_map,
        comment,
        current_code_column,
        current_indent_column,
    ) {
        return padding;
    }
    trailing_comment_padding(
        source,
        source_map,
        comment,
        current_code_column,
        current_indent_column,
    )
}

fn aligned_close_suffix_comment_padding(
    source: &str,
    source_map: &SourceMap<'_>,
    comment: &SourceComment<'_>,
    current_code_column: usize,
    current_indent_column: usize,
) -> Option<usize> {
    let entries = close_suffix_comment_alignment_entries(source, source_map, comment)?;
    if entries.len() <= 1 {
        return None;
    }
    let current = entries.first()?;
    let source_indent_unit = if current.source_indent > 0 && current_indent_column > 0 {
        current.source_indent / current_indent_column
    } else {
        entries
            .iter()
            .filter_map(|entry| (entry.source_indent > 0).then_some(entry.source_indent))
            .min()
            .unwrap_or(1)
    }
    .max(1);
    let target_column = entries
        .iter()
        .map(|entry| {
            rendered_close_suffix_source_indent(
                entry.source_indent,
                current.source_indent,
                current_indent_column,
                source_indent_unit,
            ) + entry.code_width
        })
        .max()?
        + 1;
    Some(
        target_column
            .saturating_sub(current_indent_column + current_code_column)
            .max(1),
    )
}

#[derive(Clone, Copy)]
struct CloseSuffixAlignmentEntry {
    source_indent: usize,
    code_width: usize,
}

fn close_suffix_comment_alignment_entries(
    source: &str,
    source_map: &SourceMap<'_>,
    comment: &SourceComment<'_>,
) -> Option<Vec<CloseSuffixAlignmentEntry>> {
    let (line_start, line_end) = source_map.line_bounds_for_offset(comment.span().start.offset)?;
    let mut entries = vec![close_suffix_alignment_entry(
        source,
        line_start,
        line_end,
        Some(comment.span().start.offset),
    )?];

    let mut previous_start = line_start;
    while let Some((start, end)) = source_map.previous_line_bounds(previous_start) {
        let Some(entry) = close_suffix_alignment_entry(source, start, end, None) else {
            break;
        };
        entries.push(entry);
        previous_start = start;
    }

    let mut next_line = source_map.next_line_bounds(line_end);
    while let Some((start, end)) = next_line {
        let Some(entry) = close_suffix_alignment_entry(source, start, end, None) else {
            break;
        };
        entries.push(entry);
        next_line = source_map.next_line_bounds(end);
    }

    Some(entries)
}

fn close_suffix_alignment_entry(
    source: &str,
    line_start: usize,
    line_end: usize,
    known_comment_offset: Option<usize>,
) -> Option<CloseSuffixAlignmentEntry> {
    let comment_offset = known_comment_offset
        .or_else(|| find_inline_comment_start(source.get(line_start..line_end)?, line_start))?;
    let prefix = source.get(line_start..comment_offset)?;
    let code = prefix.trim_matches([' ', '\t', '\r']);
    if !matches!(code, "fi" | "done" | "esac" | "}") {
        return None;
    }
    let (source_indent, _) = leading_indent_and_code_start(prefix)?;
    Some(CloseSuffixAlignmentEntry {
        source_indent,
        code_width: normalized_comment_alignment_width(code),
    })
}

fn rendered_close_suffix_source_indent(
    source_indent: usize,
    current_source_indent: usize,
    current_rendered_indent: usize,
    source_indent_unit: usize,
) -> usize {
    if source_indent == current_source_indent {
        return current_rendered_indent;
    }
    if current_source_indent == 0 || current_rendered_indent == 0 {
        return source_indent / source_indent_unit;
    }
    source_indent.saturating_mul(current_rendered_indent) / current_source_indent
}

fn leading_indent_and_code_start(text: &str) -> Option<(usize, usize)> {
    let mut indent_width = 0;
    for (index, ch) in text.char_indices() {
        match ch {
            ' ' | '\t' => indent_width += 1,
            '\r' => {}
            _ => return Some((indent_width, index)),
        }
    }
    None
}

fn trailing_comment_alignment_column(
    source: &str,
    source_map: &SourceMap<'_>,
    comment: &SourceComment<'_>,
    current_indent_column: usize,
) -> Option<usize> {
    let (line_start, line_end) = source_map.line_bounds_for_offset(comment.span().start.offset)?;
    let mut widths = vec![trimmed_line_width(
        source.get(line_start..comment.span().start.offset)?,
    )?];

    let mut previous_start = line_start;
    while let Some((start, end)) = source_map.previous_line_bounds(previous_start) {
        let Some(width) = inline_comment_code_width(source, start, end, None) else {
            if source
                .get(start..end)
                .is_some_and(line_is_skippable_alignment_opener)
            {
                previous_start = start;
                continue;
            }
            break;
        };
        widths.push(width);
        previous_start = start;
    }

    let mut next_line = source_map.next_line_bounds(line_end);
    while let Some((start, end)) = next_line {
        let Some(width) = inline_comment_code_width(source, start, end, None) else {
            if let Some((width, suffix_end)) = multiline_header_suffix_comment_width(
                source,
                source_map,
                line_start,
                start,
                end,
                current_indent_column,
            ) {
                widths.push(width);
                next_line = source_map.next_line_bounds(suffix_end);
                continue;
            }
            break;
        };
        widths.push(width);
        next_line = source_map.next_line_bounds(end);
    }

    (widths.len() > 1).then(|| widths.into_iter().max().unwrap_or(0) + 1)
}

fn trailing_comment_tab_indent_adjust(
    source: &str,
    source_map: &SourceMap<'_>,
    comment: &SourceComment<'_>,
) -> usize {
    let Some((line_start, line_end)) =
        source_map.line_bounds_for_offset(comment.span().start.offset)
    else {
        return 0;
    };
    let Some(current_line) = source.get(line_start..comment.span().start.offset) else {
        return 0;
    };
    let current_tabs = leading_tabs_only_indent_width(current_line);
    if current_tabs == 0 {
        return 0;
    }

    let mut previous_start = line_start;
    while let Some((start, end)) = source_map.previous_line_bounds(previous_start) {
        if inline_comment_code_width(source, start, end, None).is_some() {
            return source
                .get(start..end)
                .map(leading_tabs_only_indent_width)
                .map_or(0, |previous_tabs| {
                    if previous_tabs == 0 {
                        0
                    } else {
                        current_tabs.saturating_sub(previous_tabs)
                    }
                });
        }
        if source
            .get(start..end)
            .is_some_and(line_is_skippable_alignment_opener)
        {
            previous_start = start;
            continue;
        }
        break;
    }

    let mut next_line = source_map.next_line_bounds(line_end);
    while let Some((start, end)) = next_line {
        if inline_comment_code_width(source, start, end, None).is_some() {
            return source
                .get(start..end)
                .map(leading_tabs_only_indent_width)
                .map_or(0, |next_tabs| {
                    if next_tabs == 0 {
                        0
                    } else {
                        current_tabs.saturating_sub(next_tabs)
                    }
                });
        }
        if source
            .get(start..end)
            .is_some_and(line_is_skippable_alignment_opener)
        {
            next_line = source_map.next_line_bounds(end);
            continue;
        }
        break;
    }
    0
}

fn leading_tabs_only_indent_width(line: &str) -> usize {
    let mut tabs = 0;
    for ch in line.chars() {
        match ch {
            '\t' => tabs += 1,
            ' ' | '\r' => return 0,
            _ => break,
        }
    }
    tabs
}

fn inline_comment_code_width(
    source: &str,
    line_start: usize,
    line_end: usize,
    known_comment_offset: Option<usize>,
) -> Option<usize> {
    let comment_offset = known_comment_offset
        .or_else(|| find_inline_comment_start(source.get(line_start..line_end)?, line_start))?;
    let prefix = source.get(line_start..comment_offset)?;
    if line_is_skippable_alignment_opener(prefix) {
        return None;
    }
    trimmed_line_width(prefix)
}

fn multiline_header_suffix_comment_width(
    source: &str,
    source_map: &SourceMap<'_>,
    current_line_start: usize,
    header_start: usize,
    header_end: usize,
    current_indent_column: usize,
) -> Option<(usize, usize)> {
    let header_line = source.get(header_start..header_end)?;
    if find_inline_comment_start(header_line, header_start).is_some() {
        return None;
    }
    let header = header_line.trim_matches([' ', '\t', '\r']);
    let suffix = multiline_header_suffix_keyword(header)?;

    let (suffix_start, suffix_end) = source_map.next_line_bounds(header_end)?;
    let suffix_line = source.get(suffix_start..suffix_end)?;
    let comment_offset = find_inline_comment_start(suffix_line, suffix_start)?;
    let suffix_prefix = source.get(suffix_start..comment_offset)?;
    if suffix_prefix.trim_matches([' ', '\t', '\r']) != suffix {
        return None;
    }

    let header = header.trim_end_matches(';').trim_end();
    let rendered = format!("{header}; {suffix}");
    Some((
        alignment_width_relative_to_current_indent(
            source,
            source_map,
            current_line_start,
            header_start,
            normalized_comment_alignment_width(&rendered),
            current_indent_column,
        ),
        suffix_end,
    ))
}

fn multiline_header_suffix_keyword(header: &str) -> Option<&'static str> {
    match header.split_whitespace().next()? {
        "if" | "elif" => Some("then"),
        "for" | "select" | "until" | "while" => Some("do"),
        _ => None,
    }
}

fn alignment_width_relative_to_current_indent(
    source: &str,
    source_map: &SourceMap<'_>,
    current_line_start: usize,
    target_line_start: usize,
    width: usize,
    current_indent_column: usize,
) -> usize {
    let Some((_, current_line_end)) = source_map.line_bounds_for_offset(current_line_start) else {
        return width;
    };
    let Some((_, target_line_end)) = source_map.line_bounds_for_offset(target_line_start) else {
        return width;
    };
    let current_indent = line_indent_info(source, current_line_start, current_line_end);
    let target_indent = line_indent_info(source, target_line_start, target_line_end);
    if current_indent.has_tabs || target_indent.has_tabs {
        return width;
    }
    if let Some(adjusted) = list_rhs_to_branch_header_alignment_width(
        source,
        current_line_start,
        current_line_end,
        target_line_start,
        target_line_end,
        width,
    ) {
        return adjusted;
    }
    let delta = rendered_indent_delta_between_lines(
        source,
        source_map,
        current_line_start,
        target_line_start,
        current_indent_column,
    );
    if delta >= 0 {
        width.saturating_add(delta as usize)
    } else {
        width.saturating_sub(delta.unsigned_abs())
    }
}

fn rendered_indent_delta_between_lines(
    source: &str,
    source_map: &SourceMap<'_>,
    current_line_start: usize,
    target_line_start: usize,
    current_rendered_indent: usize,
) -> isize {
    let Some((_, current_line_end)) = source_map.line_bounds_for_offset(current_line_start) else {
        return 0;
    };
    let Some((_, target_line_end)) = source_map.line_bounds_for_offset(target_line_start) else {
        return 0;
    };
    let current_indent = line_indent_info(source, current_line_start, current_line_end);
    let target_indent = line_indent_info(source, target_line_start, target_line_end);
    if target_indent == current_indent {
        return 0;
    }
    let target_rendered_indent = rendered_source_indent_relative_to_current(
        target_indent.width,
        current_indent.width,
        current_rendered_indent,
    );
    target_rendered_indent as isize - current_rendered_indent as isize
}

fn rendered_source_indent_relative_to_current(
    target_source_indent: usize,
    current_source_indent: usize,
    current_rendered_indent: usize,
) -> usize {
    if target_source_indent == current_source_indent {
        return current_rendered_indent;
    }
    if current_source_indent == 0 || current_rendered_indent == 0 {
        let delta = target_source_indent.abs_diff(current_source_indent);
        let unit = if current_source_indent == 0 {
            delta
        } else {
            current_source_indent.min(delta)
        }
        .max(1);
        return target_source_indent / unit;
    }

    let scaled = target_source_indent.saturating_mul(current_rendered_indent);
    if target_source_indent < current_source_indent {
        scaled / current_source_indent
    } else {
        scaled.div_ceil(current_source_indent)
    }
}

fn list_rhs_to_branch_header_alignment_width(
    source: &str,
    current_line_start: usize,
    current_line_end: usize,
    target_line_start: usize,
    target_line_end: usize,
    width: usize,
) -> Option<usize> {
    let current_code = source
        .get(current_line_start..current_line_end)?
        .trim_start_matches([' ', '\t', '\r']);
    if !current_code.starts_with("||") && !current_code.starts_with("&&") {
        return None;
    }

    let target_code = source
        .get(target_line_start..target_line_end)?
        .trim_start_matches([' ', '\t', '\r']);
    target_code
        .starts_with("elif ")
        .then(|| width.saturating_sub(2))
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct LineIndentInfo {
    width: usize,
    has_tabs: bool,
}

fn line_indent_info(source: &str, line_start: usize, line_end: usize) -> LineIndentInfo {
    source
        .get(line_start..line_end)
        .map(|line| {
            let indent = line_leading_indent(line);
            LineIndentInfo {
                width: indent.chars().count(),
                has_tabs: indent.contains('\t'),
            }
        })
        .unwrap_or(LineIndentInfo {
            width: 0,
            has_tabs: false,
        })
}

fn line_leading_indent(line: &str) -> &str {
    let end = line
        .char_indices()
        .find(|(_, ch)| !matches!(ch, ' ' | '\t' | '\r'))
        .map_or(line.len(), |(index, _)| index);
    &line[..end]
}

fn find_inline_comment_start(line: &str, line_start: usize) -> Option<usize> {
    for (index, ch) in line.char_indices() {
        if ch != '#' {
            continue;
        }
        let prefix = &line[..index];
        if prefix.trim().is_empty() {
            return None;
        }
        if prefix
            .chars()
            .last()
            .is_some_and(|ch| ch == ' ' || ch == '\t')
        {
            return Some(line_start + index);
        }
    }
    None
}

fn trimmed_line_width(text: &str) -> Option<usize> {
    let trimmed = text
        .trim_start_matches([' ', '\t', '\r'])
        .trim_end_matches([' ', '\t', '\r']);
    (!trimmed.trim().is_empty()).then(|| normalized_comment_alignment_width(trimmed))
}

fn normalized_comment_alignment_width(text: &str) -> usize {
    let collapsed = collapse_horizontal_whitespace_runs(text);
    let semicolon_normalized = trim_trailing_semicolon_for_alignment(&collapsed);
    let redirect_normalized = trim_redirect_padding_for_alignment(&semicolon_normalized);
    let array_normalized = trim_compound_assignment_padding_for_alignment(&redirect_normalized);
    let parameter_normalized =
        normalize_empty_parameter_replacements_for_alignment(&array_normalized);
    let normalized = trim_arithmetic_expansion_padding_for_alignment(&parameter_normalized);
    normalized.chars().count()
        + case_pattern_pipe_alignment_width(&normalized)
        + moved_function_brace_alignment_width(&normalized)
}

fn trim_trailing_semicolon_for_alignment(text: &str) -> String {
    let trimmed = text.trim_end_matches([' ', '\t', '\r']);
    let Some(without_semicolon) = trimmed.strip_suffix(';') else {
        return text.to_string();
    };
    if without_semicolon.ends_with(';') || without_semicolon.trim().is_empty() {
        return text.to_string();
    }
    without_semicolon
        .trim_end_matches([' ', '\t', '\r'])
        .to_string()
}

fn case_pattern_pipe_alignment_width(text: &str) -> usize {
    let Some(close_paren) = text.find(')') else {
        return 0;
    };
    let pattern = &text[..close_paren];
    let mut adjustment = 0;
    for (index, ch) in pattern.char_indices() {
        if ch != '|' {
            continue;
        }
        let previous = pattern[..index].chars().next_back();
        if previous == Some('\\') {
            continue;
        }
        if previous.is_some_and(|ch| !ch.is_whitespace() && ch != '|') {
            adjustment += 1;
        }
        if pattern[index + ch.len_utf8()..]
            .chars()
            .next()
            .is_some_and(|ch| !ch.is_whitespace() && ch != '|')
        {
            adjustment += 1;
        }
    }
    adjustment
}

fn trim_compound_assignment_padding_for_alignment(text: &str) -> String {
    let mut normalized = text.to_string();
    let Some(open) = normalized.find("=(") else {
        return normalized;
    };

    let body_start = open + 2;
    while normalized
        .as_bytes()
        .get(body_start)
        .is_some_and(u8::is_ascii_whitespace)
    {
        normalized.remove(body_start);
    }

    let Some(close) = normalized.rfind(')') else {
        return normalized;
    };
    let mut index = close;
    while index > body_start
        && normalized
            .as_bytes()
            .get(index.saturating_sub(1))
            .is_some_and(u8::is_ascii_whitespace)
    {
        normalized.remove(index - 1);
        index -= 1;
    }
    normalized
}

fn moved_function_brace_alignment_width(text: &str) -> usize {
    let trimmed = text.trim_end();
    if trimmed
        .strip_prefix("function ")
        .is_some_and(|rest| !rest.trim().is_empty())
    {
        return 1;
    }
    usize::from(trimmed.ends_with("()") && !trimmed.ends_with(" ()") && !trimmed.contains('='))
}

fn line_is_skippable_alignment_opener(line: &str) -> bool {
    matches!(line.trim_matches([' ', '\t', '\r']), "{" | "(")
}

fn collapse_horizontal_whitespace_runs(text: &str) -> String {
    let mut collapsed = String::with_capacity(text.len());
    let mut in_horizontal_space = false;
    for ch in text.chars() {
        if matches!(ch, ' ' | '\t' | '\r') {
            if !in_horizontal_space {
                collapsed.push(' ');
                in_horizontal_space = true;
            }
        } else {
            collapsed.push(ch);
            in_horizontal_space = false;
        }
    }
    collapsed
}

fn trim_redirect_padding_for_alignment(text: &str) -> String {
    let mut rendered = String::with_capacity(text.len());
    let mut last = 0;
    let mut index = 0;
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut escaped = false;
    let bytes = text.as_bytes();

    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'\'' && !in_double_quotes && !escaped {
            in_single_quotes = !in_single_quotes;
            index += 1;
            continue;
        }
        if byte == b'"' && !in_single_quotes && !escaped {
            in_double_quotes = !in_double_quotes;
            index += 1;
            continue;
        }

        if !in_single_quotes && !in_double_quotes && byte.is_ascii_digit() {
            let fd_start = index;
            let mut operator_start = index + 1;
            while operator_start < bytes.len() && bytes[operator_start].is_ascii_digit() {
                operator_start += 1;
            }
            if let Some(operator_end) = redirect_operator_end(bytes, operator_start)
                && let Some(target_start) = redirect_target_start_after_padding(bytes, operator_end)
            {
                rendered.push_str(&text[last..operator_end]);
                last = target_start;
                index = target_start;
                escaped = false;
                continue;
            }
            index = fd_start;
        }

        if !in_single_quotes
            && !in_double_quotes
            && matches!(byte, b'<' | b'>')
            && !alignment_operator_is_inside_test_expression(text, index)
            && let Some(operator_end) = redirect_operator_end(bytes, index)
            && let Some(target_start) = redirect_target_start_after_padding(bytes, operator_end)
        {
            rendered.push_str(&text[last..operator_end]);
            last = target_start;
            index = target_start;
            escaped = false;
            continue;
        }

        if !in_single_quotes
            && !in_double_quotes
            && bytes.get(index..index + 3) == Some(b"<<<")
            && let Some(target_start) = redirect_target_start_after_padding(bytes, index + 3)
        {
            rendered.push_str(&text[last..index + 3]);
            last = target_start;
            index = target_start;
            escaped = false;
            continue;
        }

        escaped = !in_single_quotes && byte == b'\\' && !escaped;
        if byte != b'\\' {
            escaped = false;
        }
        index += 1;
    }

    rendered.push_str(&text[last..]);
    rendered
}

fn alignment_operator_is_inside_test_expression(text: &str, index: usize) -> bool {
    let Some(prefix) = text.get(..index) else {
        return false;
    };
    let Some(suffix) = text.get(index..) else {
        return false;
    };
    let inside_conditional = prefix
        .rfind("[[")
        .is_some_and(|open| !prefix[open + 2..].contains("]]") && suffix.contains("]]"));
    let inside_arithmetic = prefix
        .rfind("((")
        .is_some_and(|open| !prefix[open + 2..].contains("))") && suffix.contains("))"));
    inside_conditional || inside_arithmetic
}

fn redirect_target_start_after_padding(bytes: &[u8], operator_end: usize) -> Option<usize> {
    let mut target_start = operator_end;
    while target_start < bytes.len() && matches!(bytes[target_start], b' ' | b'\t' | b'\r') {
        target_start += 1;
    }
    (target_start > operator_end
        && target_start < bytes.len()
        && !matches!(bytes[target_start], b'<' | b'>' | b'&'))
    .then_some(target_start)
}

fn trim_arithmetic_expansion_padding_for_alignment(text: &str) -> String {
    let mut rendered = String::with_capacity(text.len());
    let mut index = 0;
    while index < text.len() {
        let rest = &text[index..];
        if rest.starts_with("$((") {
            rendered.push_str("$((");
            index += 3;
            while text[index..].starts_with(' ') {
                index += 1;
            }
            continue;
        }
        if rest.starts_with("$(") {
            rendered.push_str("$(");
            index += 2;
            while text[index..].starts_with(' ') {
                index += 1;
            }
            continue;
        }
        if rest.starts_with(" ))") {
            rendered.push_str("))");
            index += 3;
            continue;
        }
        if rest.starts_with(" )") {
            rendered.push(')');
            index += 2;
            continue;
        }
        let Some(ch) = rest.chars().next() else {
            break;
        };
        rendered.push(ch);
        index += ch.len_utf8();
    }
    rendered
}

fn normalize_empty_parameter_replacements_for_alignment(text: &str) -> String {
    normalize_raw_empty_parameter_replacement_delimiters(text).unwrap_or_else(|| text.to_string())
}

fn sequence_verbatim_span(statements: &StmtSeq, source_map: &SourceMap<'_>) -> Option<Span> {
    statements
        .iter()
        .map(|stmt| stmt_verbatim_span_with_source_map(stmt, source_map))
        .reduce(Span::merge)
}

fn multiline_compound_assignment_line_extra_indent(
    line: &str,
    closes_inline_assignment: bool,
) -> usize {
    if line.is_empty() {
        return 0;
    }
    if closes_inline_assignment && line == ")" {
        return 0;
    }
    if closes_inline_assignment
        && let Some(rest) = line.strip_prefix(')')
        && !rest.is_empty()
        && !rest.starts_with([' ', '\t'])
    {
        return 0;
    }
    1
}

fn case_prefix_comment_uses_body_indent(
    source: &str,
    source_map: &SourceMap<'_>,
    comment: &SourceComment<'_>,
    pattern_start: usize,
    disabled_case_pattern_context: bool,
    body_indent_context: bool,
) -> bool {
    let Some(comment_indent) = source_map.line_indent_before_offset(comment.span().start.offset)
    else {
        return false;
    };
    let Some(pattern_indent) = source_map.line_indent_before_offset(pattern_start) else {
        return false;
    };
    let comment_width = shell_indent_width(comment_indent);
    let pattern_width = shell_indent_width(pattern_indent);
    if comment_looks_like_disabled_case_pattern(comment) || disabled_case_pattern_context {
        if body_indent_context {
            return true;
        }
        if comment_width != pattern_width
            && case_prefix_comment_follows_terminator(source, source_map, comment)
        {
            return true;
        }
        return comment_text_after_hash_starts_with_tab(comment) && comment_width < pattern_width;
    }
    if comment_width < pattern_width
        && case_prefix_comment_follows_terminator(source, source_map, comment)
    {
        return true;
    }
    comment_width > pattern_width || (comment_width == 0 && pattern_width > 0)
}

fn case_prefix_comment_follows_terminator(
    source: &str,
    source_map: &SourceMap<'_>,
    comment: &SourceComment<'_>,
) -> bool {
    let Some((line_start, _)) = source_map.line_bounds_for_offset(comment.span().start.offset)
    else {
        return false;
    };
    let Some((previous_start, previous_end)) = source_map.previous_line_bounds(line_start) else {
        return false;
    };
    source
        .get(previous_start..previous_end)
        .is_some_and(|line| line.trim_end_matches([' ', '\t', '\r']).ends_with(";;"))
}

fn comment_looks_like_disabled_case_pattern(comment: &SourceComment<'_>) -> bool {
    let text = comment.text().trim_start_matches([' ', '\t']);
    let Some(rest) = text.strip_prefix('#') else {
        return false;
    };
    let rest = rest.trim_start_matches([' ', '\t']);
    let Some(close_index) = rest.find(')') else {
        return false;
    };
    let pattern = rest[..close_index].trim();
    !pattern.is_empty() && !pattern.chars().any(char::is_whitespace)
}

fn comment_text_after_hash_starts_with_tab(comment: &SourceComment<'_>) -> bool {
    let text = comment.text().trim_start_matches([' ', '\t']);
    text.strip_prefix('#')
        .is_some_and(|rest| rest.starts_with('\t'))
}

fn shell_indent_width(indent: &str) -> usize {
    indent.chars().count()
}

fn comment_precedes_close_keyword_at_same_indent(
    source: &str,
    source_map: &SourceMap<'_>,
    comment: &SourceComment<'_>,
) -> bool {
    let Some(comment_indent) = source_map.line_indent_before_offset(comment.span().start.offset)
    else {
        return false;
    };
    let Some((_, comment_line_end)) = source_map.line_bounds_for_offset(comment.span().end.offset)
    else {
        return false;
    };
    let mut next_line = source_map.next_line_bounds(comment_line_end);

    while let Some((line_start, line_end)) = next_line {
        let Some(line) = source.get(line_start..line_end) else {
            return false;
        };
        let trimmed = line.trim_start_matches([' ', '\t']);
        if trimmed.trim().is_empty() {
            next_line = source_map.next_line_bounds(line_end);
            continue;
        }
        let indent_len = line.len() - trimmed.len();
        if line.get(..indent_len) == Some(comment_indent) {
            if starts_with_outdent_preserving_close_keyword(trimmed) {
                return true;
            }
            if trimmed.starts_with('#') {
                next_line = source_map.next_line_bounds(line_end);
                continue;
            }
        }
        return false;
    }
    false
}

fn starts_with_outdent_preserving_close_keyword(text: &str) -> bool {
    ["fi"].iter().any(|keyword| {
        text.strip_prefix(keyword).is_some_and(|rest| {
            rest.chars()
                .next()
                .is_none_or(|ch| !ch.is_ascii_alphanumeric() && ch != '_')
        })
    })
}

fn stmt_is_pipeline(stmt: &Stmt) -> bool {
    matches!(
        &stmt.command,
        Command::Binary(command) if matches!(command.op, BinaryOp::Pipe | BinaryOp::PipeAll)
    )
}

fn stmt_is_redirect_only(stmt: &Stmt, source: &str) -> bool {
    matches!(
        &stmt.command,
        Command::Simple(command)
            if command.assignments.is_empty()
                && command.args.is_empty()
                && stmt_source_starts_with_redirect(stmt, source)
    )
}

fn stmt_source_starts_with_redirect(stmt: &Stmt, source: &str) -> bool {
    let text = stmt_span(stmt)
        .slice(source)
        .trim_start_matches([' ', '\t']);
    let bytes = text.as_bytes();
    let mut index = 0;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    matches!(bytes.get(index), Some(b'<' | b'>'))
}

#[derive(Clone, Copy)]
enum MultilineLiteralQuote {
    Single,
    Double,
}

fn multiline_literal_quote_state_after_line(
    line: &str,
    mut quote: Option<MultilineLiteralQuote>,
) -> Option<MultilineLiteralQuote> {
    let mut escaped = false;
    for ch in line.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match quote {
            Some(MultilineLiteralQuote::Single) => {
                if ch == '\'' {
                    quote = None;
                }
            }
            Some(MultilineLiteralQuote::Double) => {
                if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    quote = None;
                }
            }
            None => {
                if ch == '\'' {
                    quote = Some(MultilineLiteralQuote::Single);
                } else if ch == '"' {
                    quote = Some(MultilineLiteralQuote::Double);
                } else if ch == '\\' {
                    escaped = true;
                }
            }
        }
    }
    quote
}

fn pipeline_operator_breaks(
    statements: &[&Stmt],
    operators: &[(BinaryOp, Span)],
    source: &str,
    source_map: &SourceMap<'_>,
) -> Vec<bool> {
    let mut breaks = Vec::with_capacity(operators.len());
    for (statement, (_, operator_span)) in statements.iter().skip(1).zip(operators.iter()) {
        let next_start =
            interstitial_comment_end(statement, operator_span.end.offset, source, source_map);
        breaks.push(
            source_map.operator_starts_or_ends_line(*operator_span)
                || source_map.contains_newline_between(operator_span.end.offset, next_start),
        );
    }

    breaks
}

fn command_substitution_assignment_line_needs_context_indent(
    remaining: &str,
    options: &ResolvedShellFormatOptions,
) -> bool {
    match options.indent_style() {
        IndentStyle::Tab => !remaining.starts_with(' '),
        IndentStyle::Space => true,
    }
}

fn command_substitution_assignment_line_closes_block(remaining: &str) -> bool {
    remaining
        .lines()
        .next()
        .is_some_and(|line| line.trim_start_matches([' ', '\t']).starts_with(')'))
}

fn interstitial_comment_end(
    stmt: &Stmt,
    operator_end: usize,
    source: &str,
    source_map: &SourceMap<'_>,
) -> usize {
    stmt_start_after_operator(stmt, operator_end, source, source_map)
}

fn loop_condition_has_multiple_commands(condition: &StmtSeq) -> bool {
    condition.len() > 1
}

fn emitted_line_indent_column(
    line: &str,
    pipeline_indent_column: Option<usize>,
    add_context_indent: bool,
    base_indent_column: usize,
    options: &ResolvedShellFormatOptions,
) -> usize {
    pipeline_indent_column.unwrap_or_else(|| {
        let line_indent = rendered_line_indent_column(line, options);
        if add_context_indent {
            base_indent_column + line_indent
        } else {
            line_indent
        }
    })
}

fn rendered_line_indent_column(line: &str, options: &ResolvedShellFormatOptions) -> usize {
    let mut column = 0;
    for ch in line.chars() {
        match ch {
            '\t' if matches!(options.indent_style(), IndentStyle::Tab) => column += 1,
            ' ' => column += 1,
            _ => break,
        }
    }
    column
}

fn rendered_line_with_indent_column(
    line: &str,
    column: usize,
    options: &ResolvedShellFormatOptions,
) -> String {
    let content = line.trim_start_matches([' ', '\t']);
    let mut rendered = String::with_capacity(line.len());
    options.push_indent_columns(&mut rendered, column);
    rendered.push_str(content);
    rendered
}

fn command_substitution_shell_text_indent_column(
    line: &str,
    line_starts_in_quote: bool,
    emitted_indent_column: usize,
    base_indent_column: usize,
    indent_unit: usize,
) -> Option<usize> {
    if line_starts_in_quote {
        return None;
    }
    let content = line.trim_end_matches(['\r', '\n']);
    let scan_start = command_substitution_context_start(content).unwrap_or(0);
    let trimmed = content[scan_start..].trim_start_matches([' ', '\t']);
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    if inline_assignment_command_substitution_context(content, scan_start) {
        Some(base_indent_column + indent_unit)
    } else {
        Some(emitted_indent_column)
    }
}

fn inline_assignment_command_substitution_context(content: &str, scan_start: usize) -> bool {
    if scan_start == 0 {
        return false;
    }
    let prefix = content[..scan_start.saturating_sub(2)].trim_end_matches([' ', '\t']);
    prefix.ends_with('"') && prefix.contains('=')
}

fn next_command_substitution_pipeline_indent_column(
    continuation: CommandSubstitutionPipelineContinuation,
    starts_with_block_command_substitution: bool,
    inline_pipeline_indent_column: usize,
    active_shell_pipeline_indent_column: Option<usize>,
    active_shell_line_was_pipeline_stage: bool,
    indent_unit: usize,
    current_pipeline_indent_column: Option<usize>,
) -> Option<usize> {
    match continuation {
        CommandSubstitutionPipelineContinuation::None => None,
        CommandSubstitutionPipelineContinuation::Comment => current_pipeline_indent_column,
        CommandSubstitutionPipelineContinuation::StructuralPipe {
            line_started_in_quote,
        } => {
            if !starts_with_block_command_substitution {
                Some(inline_pipeline_indent_column)
            } else if line_started_in_quote && active_shell_line_was_pipeline_stage {
                active_shell_pipeline_indent_column
            } else if line_started_in_quote {
                active_shell_pipeline_indent_column.map(|column| column + indent_unit)
            } else {
                None
            }
        }
    }
}

fn strip_assignment_context_indent<'a>(
    line: &'a str,
    base_indent_column: usize,
    options: &ResolvedShellFormatOptions,
) -> &'a str {
    if base_indent_column == 0 {
        return line;
    }

    match options.indent_style() {
        IndentStyle::Tab => {
            let leading_tabs = line.bytes().take_while(|byte| *byte == b'\t').count();
            if leading_tabs <= base_indent_column {
                line
            } else {
                &line[base_indent_column..]
            }
        }
        IndentStyle::Space => {
            let leading_spaces = line.bytes().take_while(|byte| *byte == b' ').count();
            if leading_spaces <= base_indent_column {
                line
            } else {
                &line[base_indent_column..]
            }
        }
    }
}

fn normalize_literal_assignment_command_substitution_pipelines(
    text: &str,
    continuation_indent: &str,
) -> String {
    let mut output = String::with_capacity(text.len());
    let mut indent_next = false;
    let mut changed = false;
    let mut rest = text;

    while !rest.is_empty() {
        let (line, next, had_newline) = split_first_line(rest);

        let trimmed_start = line.trim_start_matches([' ', '\t']);
        let is_continuation_comment = indent_next && trimmed_start.starts_with('#');
        let indent_line = indent_next && !line.trim_matches([' ', '\t', '\r']).is_empty();

        if indent_line {
            output.push_str(continuation_indent);
            output.push_str(trimmed_start);
            changed |= !line.starts_with(continuation_indent)
                || line[continuation_indent.len()..].starts_with([' ', '\t']);
        } else {
            output.push_str(line);
        }

        if had_newline {
            output.push('\n');
        }

        let line_to_check = if indent_line { trimmed_start } else { line };
        indent_next = if is_continuation_comment {
            true
        } else if indent_next {
            rendered_line_ends_with_structural_pipe_continuation(line_to_check)
        } else {
            rendered_line_opens_command_substitution_pipeline(line_to_check)
        };

        rest = next;
    }

    if changed { output } else { text.to_string() }
}

fn conditional_binary_has_explicit_rhs_break(
    expression: &ConditionalBinaryExpr,
    source_map: &SourceMap<'_>,
) -> bool {
    if !matches!(
        expression.op,
        ConditionalBinaryOp::And | ConditionalBinaryOp::Or
    ) {
        return false;
    }
    source_map.operator_starts_or_ends_line(expression.op_span)
        || source_map.contains_newline_between(
            expression.left.span().end.offset,
            expression.op_span.start.offset,
        )
        || source_map.contains_newline_between(
            expression.op_span.end.offset,
            expression.right.span().start.offset,
        )
}

fn conditional_expr_contains_command_substitution(expression: &ConditionalExpr) -> bool {
    match expression {
        ConditionalExpr::Binary(expr) => {
            conditional_expr_contains_command_substitution(&expr.left)
                || conditional_expr_contains_command_substitution(&expr.right)
        }
        ConditionalExpr::Unary(expr) => conditional_expr_contains_command_substitution(&expr.expr),
        ConditionalExpr::Parenthesized(expr) => {
            conditional_expr_contains_command_substitution(&expr.expr)
        }
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
            word_contains_command_substitution(word)
        }
        ConditionalExpr::Pattern(pattern) => pattern_contains_command_substitution(pattern),
        ConditionalExpr::VarRef(_) => false,
    }
}

fn pattern_contains_command_substitution(pattern: &Pattern) -> bool {
    pattern.parts.iter().any(|part| match &part.kind {
        PatternPart::Group { patterns, .. } => {
            patterns.iter().any(pattern_contains_command_substitution)
        }
        PatternPart::Word(word) => word_contains_command_substitution(word),
        PatternPart::Literal(_)
        | PatternPart::AnyString
        | PatternPart::AnyChar
        | PatternPart::CharClass(_) => false,
    })
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum WordSubstitutionKind {
    Command,
    Process,
}

fn word_contains_substitution(word: &Word, kind: WordSubstitutionKind) -> bool {
    word.parts
        .iter()
        .any(|part| word_part_contains_substitution(&part.kind, kind))
}

fn word_contains_command_substitution(word: &Word) -> bool {
    word_contains_substitution(word, WordSubstitutionKind::Command)
}

fn word_contains_process_substitution(word: &Word) -> bool {
    word_contains_substitution(word, WordSubstitutionKind::Process)
}

fn word_source_has_shell_substitution(word: &Word, source: &str) -> bool {
    let raw = word.span.slice(source);
    rendered_text_has_shell_substitution(raw)
}

fn rendered_text_has_shell_substitution(text: &str) -> bool {
    text.contains("$(") || text.contains('`') || text.contains("<(") || text.contains(">(")
}

fn rendered_text_starts_with_block_command_substitution(text: &str) -> bool {
    text.lines()
        .next()
        .is_some_and(|line| line.trim_end_matches([' ', '\t', '\r']).ends_with("$("))
}

fn rendered_text_starts_like_assignment_with_substitution(text: &str) -> bool {
    let first_line = text.lines().next().unwrap_or(text);
    let substitution_start = ["$(", "`", "<(", ">("]
        .iter()
        .filter_map(|marker| first_line.find(marker))
        .min()
        .unwrap_or(first_line.len());
    first_line[..substitution_start].contains('=')
}

fn rendered_text_has_leading_list_operator_line(text: &str) -> bool {
    text.lines().skip(1).any(|line| {
        let trimmed = line.trim_start_matches([' ', '\t', '\r']);
        (trimmed.starts_with('|') && !trimmed.starts_with("|)")) || trimmed.starts_with("&&")
    })
}

fn word_part_contains_substitution(part: &WordPart, kind: WordSubstitutionKind) -> bool {
    match part {
        WordPart::CommandSubstitution { .. } => kind == WordSubstitutionKind::Command,
        WordPart::ProcessSubstitution { .. } => kind == WordSubstitutionKind::Process,
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter()
            .any(|part| word_part_contains_substitution(&part.kind, kind)),
        WordPart::ArithmeticExpansion {
            expression_word_ast,
            ..
        } => word_contains_substitution(expression_word_ast, kind),
        WordPart::ParameterExpansion {
            operand_word_ast, ..
        } => operand_word_ast
            .as_deref()
            .is_some_and(|word| word_contains_substitution(word, kind)),
        WordPart::IndirectExpansion {
            operand_word_ast, ..
        } => operand_word_ast
            .as_deref()
            .is_some_and(|word| word_contains_substitution(word, kind)),
        WordPart::Substring {
            offset_word_ast,
            length_word_ast,
            ..
        }
        | WordPart::ArraySlice {
            offset_word_ast,
            length_word_ast,
            ..
        } => {
            word_contains_substitution(offset_word_ast, kind)
                || length_word_ast
                    .as_deref()
                    .is_some_and(|word| word_contains_substitution(word, kind))
        }
        _ => false,
    }
}

fn collect_command_list_first<'a>(
    command: &'a BinaryCommand,
    rest: &mut Vec<BinaryListItem<'a>>,
) -> &'a Stmt {
    collect_binary_list_first_with(command, rest, &|command| BinaryListItem {
        operator: command.op,
        operator_span: command.op_span,
        stmt: command.right.as_ref(),
    })
}

fn list_item_separator(operator: BinaryOp, inline: bool) -> &'static str {
    match (operator, inline) {
        (BinaryOp::And, true) => " && ",
        (BinaryOp::And, false) => " &&",
        (BinaryOp::Or, true) => " || ",
        (BinaryOp::Or, false) => " ||",
        (BinaryOp::Pipe | BinaryOp::PipeAll, true) => "; ",
        (BinaryOp::Pipe | BinaryOp::PipeAll, false) => ";",
    }
}

fn if_branch_upper_bound(
    command: &IfCommand,
    branch_index: usize,
    source: &str,
    source_map: &SourceMap<'_>,
    facts: &FormatterFacts<'_>,
) -> usize {
    if let Some((start, end)) = if_next_branch_region(command, branch_index, source, facts) {
        facts
            .branch_prefix_first_comment_offset(start, end)
            .unwrap_or(end)
    } else {
        command_if_close_span(command, source, source_map)
            .start
            .offset
    }
}

fn if_next_branch_region(
    command: &IfCommand,
    branch_index: usize,
    source: &str,
    facts: &FormatterFacts<'_>,
) -> Option<(usize, usize)> {
    if_next_branch_region_with_body_end(command, branch_index, source, |body| {
        branch_body_content_end(body, source, facts)
    })
}

fn branch_body_content_end(body: &StmtSeq, source: &str, facts: &FormatterFacts<'_>) -> usize {
    let mut end = body
        .last()
        .map(|stmt| stmt_span(stmt).end.offset)
        .unwrap_or(body.span.end.offset);
    if let Some(stmt) = body.last() {
        for redirect in &stmt.redirects {
            let Some(heredoc) = redirect.heredoc() else {
                continue;
            };
            let heredoc_end = facts
                .heredoc_closing_marker_bounds(heredoc)
                .map(|(_, line_end)| line_end)
                .unwrap_or(heredoc.body.span.end.offset);
            end = end.max(heredoc_end);
        }
    }
    let end = end.min(source.len());
    trim_trailing_gap_before_offset(source, end)
}

fn trim_trailing_gap_before_offset(source: &str, mut offset: usize) -> usize {
    let bytes = source.as_bytes();
    while offset > 0 && matches!(bytes[offset - 1], b' ' | b'\t' | b'\r' | b'\n') {
        offset -= 1;
    }
    offset
}

fn comment_looks_like_disabled_if_branch(text: &str) -> bool {
    let body = text
        .strip_prefix('#')
        .unwrap_or(text)
        .trim_start_matches([' ', '\t']);
    ["elif", "else"]
        .iter()
        .any(|keyword| shell_keyword_at(body, 0, body.len(), keyword))
}

fn branch_prefix_comments_use_disabled_body_indent(comments: &[BranchPrefixComment]) -> bool {
    let Some(first) = comments.first() else {
        return false;
    };
    comment_looks_like_disabled_if_branch(&first.text)
        && comments
            .iter()
            .skip(1)
            .any(|comment| comment.source_indent > first.source_indent)
}

fn time_inner_stmt_needs_trailing_comment(stmt: &Stmt) -> bool {
    match &stmt.command {
        Command::Simple(_)
        | Command::Builtin(_)
        | Command::Decl(_)
        | Command::Compound(CompoundCommand::Arithmetic(_) | CompoundCommand::Conditional(_)) => {
            true
        }
        Command::Binary(command) => time_inner_stmt_needs_trailing_comment(&command.right),
        _ => false,
    }
}

fn unmodeled_branch_background_operator(
    body: &StmtSeq,
    upper_bound: usize,
    source: &str,
) -> Option<&'static str> {
    let last = body.last()?;
    if matches!(last.terminator, Some(StmtTerminator::Background(_))) {
        return None;
    }

    let body_end = body.span.end.offset.min(upper_bound).min(source.len());
    let stmt_start = stmt_span(last).start.offset.min(body_end);
    if let Some(body_tail) = source.get(stmt_start..body_end)
        && let Some(operator) = trailing_unmodeled_background_operator(body_tail)
    {
        return Some(operator);
    }

    let start = stmt_span(last)
        .end
        .offset
        .min(upper_bound)
        .min(source.len());
    let end = upper_bound.min(source.len());
    let between = source.get(start..end)?;
    let trimmed = between.trim_start_matches([' ', '\t', '\r', '\n']);
    let (operator, rest) = if let Some(rest) = trimmed.strip_prefix("&|") {
        ("&|", rest)
    } else if let Some(rest) = trimmed.strip_prefix("&!") {
        ("&!", rest)
    } else if let Some(rest) = trimmed.strip_prefix('&') {
        ("&", rest)
    } else {
        return None;
    };

    rest.chars()
        .next()
        .is_none_or(|ch| matches!(ch, ' ' | '\t' | '\r' | '\n'))
        .then_some(operator)
}

fn trailing_unmodeled_background_operator(text: &str) -> Option<&'static str> {
    let trimmed = text.trim_end_matches([' ', '\t', '\r', '\n']);
    if trimmed.ends_with("&|") {
        Some("&|")
    } else if trimmed.ends_with("&!") {
        Some("&!")
    } else if trimmed.ends_with('&') && !trimmed.ends_with("&&") {
        Some("&")
    } else {
        None
    }
}
