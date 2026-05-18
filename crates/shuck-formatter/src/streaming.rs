use std::fmt::Write as _;
use std::mem;

use shuck_ast::{
    AlwaysCommand, AnonymousFunctionCommand, ArithmeticCommand, ArithmeticForCommand, ArrayElem,
    Assignment, AssignmentValue, BinaryCommand, BinaryOp, BuiltinCommand, CaseCommand, CaseItem,
    Command, CompoundCommand, ConditionalBinaryExpr, ConditionalBinaryOp, ConditionalCommand,
    ConditionalExpr, ConditionalParenExpr, ConditionalUnaryExpr, ConditionalUnaryOp, CoprocCommand,
    DeclClause, DeclOperand, File, ForCommand, ForSyntax, ForeachCommand, ForeachSyntax,
    FunctionDef, Heredoc, HeredocBody, HeredocBodyPart, IfCommand, IfSyntax, Pattern, PatternPart,
    Redirect, RedirectKind, RepeatCommand, RepeatSyntax, SelectCommand, SimpleCommand, Span, Stmt,
    StmtSeq, StmtTerminator, TimeCommand, UntilCommand, VarRef, WhileCommand, Word, WordPart,
};
use shuck_format::{IndentStyle, LineEnding};

use crate::Result;
use crate::command::{
    binary_operator, case_terminator, command_format_span, format_arithmetic_command_source,
    format_arithmetic_for_clause_source, group_attachment_span, line_gap_break_count,
    multiline_compound_assignment_layout, multiline_compound_assignment_lines,
    render_assignment_head_to_buf, render_assignment_with_facts_to_buf, render_background_operator,
    render_subscript_to_buf, render_var_ref_to_buf, slice_span, stmt_attachment_span,
    stmt_format_span, stmt_render_start_line, stmt_seq_has_heredoc, stmt_span,
    stmt_verbatim_span_with_source_map,
};
use crate::comments::{SourceComment, SourceMap};
use crate::facts::FormatterFacts;
use crate::options::ResolvedShellFormatOptions;
use crate::word::{
    normalize_raw_unquoted_word_continuations, render_arithmetic_expr_to_buf,
    render_heredoc_body_to_buf, render_pattern_syntax_to_buf, render_word_syntax_with_facts_to_buf,
    word_gap_end_before_trailing_continuation, word_has_multiline_literal_source,
    word_is_quoted_command_substitution_only, word_is_quoted_formattable_command_substitution_only,
};

enum StreamOutput<'source> {
    Buffer(String),
    Compare(CompareSink<'source>),
}

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

impl<'source> StreamOutput<'source> {
    fn push_char(&mut self, ch: char) {
        match self {
            Self::Buffer(buffer) => buffer.push(ch),
            Self::Compare(compare) => compare.push_char(ch),
        }
    }

    fn push_str(&mut self, text: &str) {
        match self {
            Self::Buffer(buffer) => buffer.push_str(text),
            Self::Compare(compare) => compare.push_str(text),
        }
    }

    fn finish_into_string(self) -> String {
        match self {
            Self::Buffer(buffer) => buffer,
            Self::Compare(_) => panic!("comparison formatter cannot yield a String"),
        }
    }

    fn finish_matches_source(mut self, line_ending: LineEnding) -> bool {
        match &mut self {
            Self::Buffer(_) => panic!("buffer formatter cannot compare against the source"),
            Self::Compare(compare) => compare.finish(line_ending),
        }
    }
}

struct CompareSink<'source> {
    source: &'source str,
    matched_bytes: usize,
    pending_tail: String,
    mismatch: bool,
}

impl<'source> CompareSink<'source> {
    fn new(source: &'source str) -> Self {
        Self {
            source,
            matched_bytes: 0,
            pending_tail: String::new(),
            mismatch: false,
        }
    }

    fn push_char(&mut self, ch: char) {
        let mut encoded = [0; 4];
        self.push_str(ch.encode_utf8(&mut encoded));
    }

    fn push_str(&mut self, text: &str) {
        if self.mismatch || text.is_empty() {
            return;
        }

        let prefix_end = text
            .bytes()
            .rposition(|byte| byte != b'\n' && byte != b'\r')
            .map_or(0, |index| index + 1);

        if !self.pending_tail.is_empty() && prefix_end > 0 {
            let pending_tail = mem::take(&mut self.pending_tail);
            self.compare_prefix(&pending_tail);
            if self.mismatch {
                return;
            }
        }

        if prefix_end > 0 {
            self.compare_prefix(&text[..prefix_end]);
            if self.mismatch {
                return;
            }
        }

        if prefix_end < text.len() {
            self.pending_tail.push_str(&text[prefix_end..]);
        }
    }

    fn finish(&mut self, line_ending: LineEnding) -> bool {
        if self.mismatch {
            return false;
        }

        crate::ensure_single_trailing_newline(&mut self.pending_tail, line_ending);
        let tail = mem::take(&mut self.pending_tail);
        self.compare_prefix(&tail);

        !self.mismatch && self.matched_bytes == self.source.len()
    }

    fn compare_prefix(&mut self, text: &str) {
        if self.mismatch || text.is_empty() {
            return;
        }

        let end = self.matched_bytes.saturating_add(text.len());
        match self.source.as_bytes().get(self.matched_bytes..end) {
            Some(candidate) if candidate == text.as_bytes() => {
                self.matched_bytes = end;
            }
            _ => self.mismatch = true,
        }
    }
}

pub(crate) fn format_file_streaming(
    source: &str,
    file: &File,
    options: &ResolvedShellFormatOptions,
) -> Result<String> {
    let facts = FormatterFacts::build(source, file, options);
    let mut formatter = ShellStreamFormatter::new(source, options, &facts);
    formatter.format_stmt_sequence(&file.body, None)?;

    Ok(formatter.finish_into_string())
}

pub(crate) fn format_file_streaming_matches_source(
    source: &str,
    file: &File,
    options: &ResolvedShellFormatOptions,
) -> Result<bool> {
    let facts = FormatterFacts::build(source, file, options);
    let mut formatter = ShellStreamFormatter::new_compare(source, options, &facts);
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

    let mut formatter =
        ShellStreamFormatter::with_output_buffer(source, options, facts, nested_output);
    formatter.format_stmt_sequence(statements, upper_bound)?;
    *output = formatter.finish_into_string();
    Ok(())
}

#[derive(Debug, Clone)]
struct PendingHeredoc {
    body: String,
    delimiter: String,
    strip_tabs: bool,
}

#[derive(Debug, Clone, Copy)]
struct BinaryListItem<'a> {
    operator: BinaryOp,
    operator_span: Span,
    stmt: &'a Stmt,
}

struct ShellStreamFormatter<'source, 'facts> {
    source: &'source str,
    options: ResolvedShellFormatOptions,
    facts: &'facts FormatterFacts<'source>,
    output: StreamOutput<'source>,
    scratch: String,
    indent_level: usize,
    column: usize,
    line_indent_column: usize,
    line_start: bool,
    pipeline_continuation_indent: usize,
    filter_next_group_body_leading_before_open: bool,
    pending_heredocs: Vec<PendingHeredoc>,
}

impl<'source, 'facts> ShellStreamFormatter<'source, 'facts> {
    fn new(
        source: &'source str,
        options: &ResolvedShellFormatOptions,
        facts: &'facts FormatterFacts<'source>,
    ) -> Self {
        Self::with_output(
            source,
            options,
            facts,
            StreamOutput::Buffer(String::with_capacity(source.len())),
        )
    }

    fn new_compare(
        source: &'source str,
        options: &ResolvedShellFormatOptions,
        facts: &'facts FormatterFacts<'source>,
    ) -> Self {
        Self::with_output(
            source,
            options,
            facts,
            StreamOutput::Compare(CompareSink::new(source)),
        )
    }

    fn with_output_buffer(
        source: &'source str,
        options: &ResolvedShellFormatOptions,
        facts: &'facts FormatterFacts<'source>,
        output: String,
    ) -> Self {
        Self::with_output(source, options, facts, StreamOutput::Buffer(output))
    }

    fn with_output(
        source: &'source str,
        options: &ResolvedShellFormatOptions,
        facts: &'facts FormatterFacts<'source>,
        output: StreamOutput<'source>,
    ) -> Self {
        Self {
            source,
            options: options.clone(),
            facts,
            output,
            scratch: String::new(),
            indent_level: 0,
            column: 0,
            line_indent_column: 0,
            line_start: true,
            pipeline_continuation_indent: 1,
            filter_next_group_body_leading_before_open: false,
            pending_heredocs: Vec::new(),
        }
    }

    fn finish_into_string(mut self) -> String {
        self.flush_pending_heredocs();
        self.output.finish_into_string()
    }

    fn finish_matches_source(mut self) -> bool {
        self.flush_pending_heredocs();
        self.output
            .finish_matches_source(self.options.line_ending())
    }

    fn push_output_char(&mut self, ch: char) {
        self.output.push_char(ch);
        self.advance_column(ch);
    }

    fn push_output_str(&mut self, text: &str) {
        self.output.push_str(text);
        for ch in text.chars() {
            self.advance_column(ch);
        }
    }

    fn advance_column(&mut self, ch: char) {
        if ch == '\n' {
            self.column = 0;
            self.line_indent_column = 0;
        } else {
            self.column = self.column.saturating_add(1);
        }
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
        match self.options.line_ending() {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
        }
    }

    fn indent_column_for_level(&self, level: usize) -> usize {
        if self.options.minify() {
            return 0;
        }
        match self.options.indent_style() {
            IndentStyle::Tab => level,
            IndentStyle::Space => level * usize::from(self.options.indent_width()),
        }
    }

    fn with_indent<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.indent_level += 1;
        let result = f(self);
        self.indent_level = self.indent_level.saturating_sub(1);
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
        if levels == 0 {
            return;
        }

        if self.line_start {
            self.write_indent();
        }

        match self.options.indent_style() {
            IndentStyle::Tab => {
                for _ in 0..levels {
                    self.push_output_char('\t');
                }
            }
            IndentStyle::Space => {
                for _ in 0..(levels * usize::from(self.options.indent_width())) {
                    self.push_output_char(' ');
                }
            }
        }

        self.line_indent_column = self.column;
        self.line_start = false;
    }

    fn write_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        let mut remaining = text;
        while !remaining.is_empty() {
            if self.line_start && !remaining.starts_with('\n') {
                self.write_indent();
            }

            match remaining.find('\n') {
                Some(index) => {
                    let end = index + 1;
                    self.push_output_str(&remaining[..end]);
                    self.line_start = true;
                    remaining = &remaining[end..];
                }
                None => {
                    self.push_output_str(remaining);
                    self.line_start = false;
                    break;
                }
            }
        }
    }

    fn write_rendered_shell_text(&mut self, text: &str) {
        if text.contains('\n') {
            if self.line_start {
                self.write_indent();
            }
            self.write_verbatim(text);
        } else {
            self.write_text(text);
        }
    }

    fn write_text_preserving_current_line_indent(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        let base_indent_column = if self.line_start {
            self.indent_column_for_level(self.indent_level)
        } else {
            self.line_indent_column
        };
        let mut remaining = text;
        while !remaining.is_empty() {
            if self.line_start && !remaining.starts_with('\n') {
                self.write_indent_to_column(base_indent_column);
            }

            match remaining.find('\n') {
                Some(index) => {
                    let end = index + 1;
                    self.push_output_str(&remaining[..end]);
                    self.line_start = true;
                    remaining = &remaining[end..];
                }
                None => {
                    self.push_output_str(remaining);
                    self.line_start = false;
                    break;
                }
            }
        }
    }

    fn write_command_substitution_assignment_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        let base_indent_column = if self.line_start {
            self.indent_column_for_level(self.indent_level)
        } else {
            self.line_indent_column
        };
        let starts_with_block_command_substitution = text
            .lines()
            .next()
            .is_some_and(|line| line.trim_end_matches([' ', '\t', '\r']).ends_with("$("));
        let strip_context_indent = !starts_with_block_command_substitution;
        let indent_unit = match self.options.indent_style() {
            IndentStyle::Tab => 1,
            IndentStyle::Space => usize::from(self.options.indent_width()),
        };
        let inline_pipeline_indent_column = base_indent_column + indent_unit;
        let mut next_pipeline_indent_column = None;
        let mut active_shell_pipeline_indent_column: Option<usize> = None;
        let mut active_shell_line_was_pipeline_stage = false;
        let mut next_block_line_is_pipeline_stage = false;
        let mut next_block_line_aligns_with_command_continuation = false;
        let mut command_continuation_active = false;
        let mut pipeline_quote_state = RenderedLineQuoteState::default();
        let mut remaining = text;
        while !remaining.is_empty() {
            let line_started_as_command_continuation = command_continuation_active;
            let pipeline_indent_column = next_pipeline_indent_column;
            let closes_block_command_substitution = starts_with_block_command_substitution
                && command_substitution_assignment_line_closes_block(remaining);
            let close_line_has_context_indent = closes_block_command_substitution
                && remaining.lines().next().is_some_and(|line| {
                    rendered_line_indent_column(line, self.options()) >= base_indent_column
                });
            let pipeline_stage_indent = self.line_start
                && !remaining.starts_with('\n')
                && pipeline_indent_column.is_some()
                && !closes_block_command_substitution
                && !remaining
                    .trim_start_matches([' ', '\t', '\r'])
                    .starts_with('\n');
            let add_context_indent = self.line_start
                && !remaining.starts_with('\n')
                && !pipeline_stage_indent
                && !close_line_has_context_indent
                && command_substitution_assignment_line_needs_context_indent(
                    remaining,
                    self.options(),
                );
            if pipeline_stage_indent {
                self.write_indent_to_column(pipeline_indent_column.unwrap_or_default());
            }
            if add_context_indent {
                self.write_indent_to_column(base_indent_column);
            }

            match remaining.find('\n') {
                Some(index) => {
                    let end = index + 1;
                    let line = &remaining[..end];
                    let line = if pipeline_stage_indent {
                        line.trim_start_matches([' ', '\t'])
                    } else if add_context_indent && strip_context_indent {
                        strip_assignment_context_indent(line, base_indent_column, self.options())
                    } else {
                        line
                    };
                    let adjusted_block_pipeline_stage;
                    let line = if starts_with_block_command_substitution
                        && next_block_line_is_pipeline_stage
                        && next_block_line_aligns_with_command_continuation
                        && !pipeline_stage_indent
                        && let Some(shell_indent_column) = active_shell_pipeline_indent_column
                    {
                        let target_column = shell_indent_column.saturating_sub(base_indent_column);
                        let line_indent_column = rendered_line_indent_column(line, self.options());
                        if line_indent_column > target_column {
                            adjusted_block_pipeline_stage = Some(rendered_line_with_indent_column(
                                line,
                                target_column,
                                self.options(),
                            ));
                            adjusted_block_pipeline_stage.as_deref().unwrap_or(line)
                        } else {
                            line
                        }
                    } else {
                        line
                    };
                    let emitted_indent_column = emitted_line_indent_column(
                        line,
                        pipeline_indent_column,
                        add_context_indent,
                        base_indent_column,
                        self.options(),
                    );
                    if let Some(shell_indent_column) = command_substitution_shell_text_indent_column(
                        line,
                        pipeline_quote_state.in_quote(),
                        emitted_indent_column,
                        base_indent_column,
                        indent_unit,
                    ) {
                        active_shell_pipeline_indent_column = Some(shell_indent_column);
                        active_shell_line_was_pipeline_stage =
                            pipeline_stage_indent || next_block_line_is_pipeline_stage;
                        next_block_line_is_pipeline_stage = false;
                        next_block_line_aligns_with_command_continuation = false;
                    }
                    self.push_output_str(line);
                    let line_continues_command = !pipeline_quote_state.in_quote()
                        && line_has_trailing_continuation_backslash(line);
                    let continuation = command_substitution_pipeline_stage_continuation(
                        line,
                        pipeline_stage_indent,
                        &mut pipeline_quote_state,
                    );
                    next_pipeline_indent_column = next_command_substitution_pipeline_indent_column(
                        continuation,
                        starts_with_block_command_substitution,
                        inline_pipeline_indent_column,
                        active_shell_pipeline_indent_column,
                        active_shell_line_was_pipeline_stage,
                        indent_unit,
                        pipeline_indent_column,
                    );
                    if matches!(
                        continuation,
                        CommandSubstitutionPipelineContinuation::StructuralPipe {
                            line_started_in_quote: false
                        }
                    ) && starts_with_block_command_substitution
                    {
                        next_block_line_is_pipeline_stage = true;
                        next_block_line_aligns_with_command_continuation =
                            line_started_as_command_continuation;
                    }
                    command_continuation_active = line_continues_command;
                    self.line_start = true;
                    remaining = &remaining[end..];
                }
                None => {
                    let line = if pipeline_stage_indent {
                        remaining.trim_start_matches([' ', '\t'])
                    } else if add_context_indent && strip_context_indent {
                        strip_assignment_context_indent(
                            remaining,
                            base_indent_column,
                            self.options(),
                        )
                    } else {
                        remaining
                    };
                    self.push_output_str(line);
                    self.line_start = false;
                    break;
                }
            }
        }
    }

    fn write_rendered_shell_text_preserving_heredoc_tails(&mut self, text: &str) {
        let mut active_heredoc: Option<RenderedHeredocTail> = None;
        let mut rest = text;

        while !rest.is_empty() {
            let (line, next, had_newline) = match rest.find('\n') {
                Some(index) => (&rest[..index], &rest[index + 1..], true),
                None => (rest, "", false),
            };

            if let Some(heredoc) = active_heredoc.as_ref() {
                self.write_verbatim(line);
                if heredoc.closes(line) {
                    active_heredoc = None;
                }
            } else if let Some(heredoc) = rendered_heredoc_tail_start(line) {
                if self.options().space_redirects() {
                    self.write_text(line);
                } else if let Some(normalized) = normalize_rendered_heredoc_start_spacing(line) {
                    self.write_text(&normalized);
                } else {
                    self.write_text(line);
                }
                active_heredoc = Some(heredoc);
            } else {
                self.write_text(line);
            }

            if had_newline {
                self.push_output_str(self.line_ending());
                self.line_start = true;
            }
            rest = next;
        }
    }

    fn write_verbatim(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.push_output_str(text);
        self.line_start = text.ends_with('\n');
    }

    fn write_indent(&mut self) {
        if !self.line_start || self.indent_level == 0 || self.options.minify() {
            return;
        }

        match self.options.indent_style() {
            IndentStyle::Tab => {
                for _ in 0..self.indent_level {
                    self.push_output_char('\t');
                }
            }
            IndentStyle::Space => {
                for _ in 0..(self.indent_level * usize::from(self.options.indent_width())) {
                    self.push_output_char(' ');
                }
            }
        }

        self.line_indent_column = self.column;
        self.line_start = false;
    }

    fn write_indent_to_column(&mut self, column: usize) {
        if !self.line_start || column == 0 || self.options.minify() {
            return;
        }

        match self.options.indent_style() {
            IndentStyle::Tab => {
                for _ in 0..column {
                    self.push_output_char('\t');
                }
            }
            IndentStyle::Space => {
                for _ in 0..column {
                    self.push_output_char(' ');
                }
            }
        }

        self.line_indent_column = self.column;
        self.line_start = false;
    }

    fn write_space(&mut self) {
        if self.line_start {
            return;
        }
        self.push_output_char(' ');
    }

    fn flush_pending_heredocs(&mut self) {
        let pending = mem::take(&mut self.pending_heredocs);
        for heredoc in pending {
            self.push_output_str(self.line_ending());
            self.line_start = true;
            if heredoc.strip_tabs {
                self.write_indented_heredoc_text(&heredoc.body);
            } else {
                self.write_verbatim(&heredoc.body);
            }
            if heredoc_body_needs_separator(&heredoc.body) {
                self.push_output_str(self.line_ending());
                self.line_start = true;
            }
            if heredoc.strip_tabs {
                self.write_indent();
                self.write_verbatim(heredoc.delimiter.trim_start_matches('\t'));
            } else {
                self.write_verbatim(&heredoc.delimiter);
            }
        }
    }

    fn write_indented_heredoc_text(&mut self, text: &str) {
        let indent_level = self.indent_level.saturating_add(1);
        let prefix = self.indent_prefix_for_level(indent_level);
        let base_tabs = if matches!(self.options.indent_style(), IndentStyle::Tab) {
            minimum_leading_tabs_in_non_empty_lines(text)
        } else {
            0
        };
        let mut rest = text;
        while !rest.is_empty() {
            let (line, next) = match rest.find('\n') {
                Some(index) => rest.split_at(index + 1),
                None => (rest, ""),
            };
            let content = line.trim_end_matches(['\r', '\n']);
            if !content.is_empty() {
                match self.options.indent_style() {
                    IndentStyle::Tab => {
                        let leading_tabs = line.bytes().take_while(|byte| *byte == b'\t').count();
                        for _ in 0..indent_level {
                            self.push_output_char('\t');
                        }
                        self.push_output_str(&line[leading_tabs.min(base_tabs)..]);
                    }
                    IndentStyle::Space => {
                        if !line.starts_with('\t') {
                            self.push_output_str(&prefix);
                        }
                        self.push_output_str(line);
                    }
                }
            } else {
                self.push_output_str(line);
            }
            self.line_start = line.ends_with('\n');
            rest = next;
        }
    }

    fn indent_prefix_for_level(&self, level: usize) -> String {
        match self.options.indent_style() {
            IndentStyle::Tab => "\t".repeat(level),
            IndentStyle::Space => " ".repeat(level * usize::from(self.options.indent_width())),
        }
    }

    fn newline(&mut self) {
        self.flush_pending_heredocs();
        self.push_output_str(self.line_ending());
        self.line_start = true;
    }

    fn line_continuation(&mut self) {
        self.flush_pending_heredocs();
        // A backslash only escapes the following LF, so CRLF here would change
        // the command structure by leaving the carriage return behind.
        self.push_output_str(" \\\n");
        self.line_start = true;
    }

    fn write_line_breaks(&mut self, count: usize) {
        for _ in 0..count {
            self.newline();
        }
    }

    fn write_word(&mut self, word: &Word) {
        let source_map = self.source_map().clone();
        let mut scratch = self.take_scratch_buffer();
        {
            let facts = self.facts();
            render_word_syntax_with_facts_to_buf(
                word,
                self.source(),
                self.options(),
                &source_map,
                facts,
                &mut scratch,
            );
        }
        if word_contains_command_heredoc(word) && rendered_shell_text_has_heredoc_tail(&scratch) {
            self.write_rendered_shell_text_preserving_heredoc_tails(&scratch);
        } else if scratch.contains('\n')
            && word_is_quoted_formattable_command_substitution_only(word, self.source())
        {
            self.write_text_preserving_current_line_indent(&scratch);
        } else if scratch.contains('\n') && word_contains_process_substitution(word) {
            self.write_text_preserving_current_line_indent(&scratch);
        } else if word_has_multiline_literal_source(word, self.source()) {
            self.write_rendered_shell_text(&scratch);
        } else if scratch.contains('\n') && word_contains_command_substitution(word) {
            self.write_text_preserving_current_line_indent(&scratch);
        } else {
            self.write_text(&scratch);
        }
        self.restore_scratch_buffer(scratch);
    }

    fn write_pattern(&mut self, pattern: &Pattern) {
        self.write_rendered(|scratch, source, options| {
            render_pattern_syntax_to_buf(pattern, source, options, scratch);
        });
    }

    fn write_case_pattern(&mut self, item: &CaseItem, pattern: &Pattern) {
        let mut scratch = self.take_scratch_buffer();
        render_pattern_syntax_to_buf(pattern, self.source(), self.options(), &mut scratch);
        if case_item_pattern_close_paren_on_own_line(item, self.source()) {
            trim_trailing_pattern_line_continuation(&mut scratch);
        }
        self.write_text(&scratch);
        self.restore_scratch_buffer(scratch);
    }

    fn write_var_ref(&mut self, reference: &VarRef) {
        self.write_rendered(|scratch, source, _| {
            render_var_ref_to_buf(reference, source, scratch);
        });
    }

    fn write_assignment(&mut self, assignment: &Assignment) {
        if assignment_has_quoted_backslash_continuation_literal(assignment, self.source()) {
            self.write_rendered_shell_text(assignment.span.slice(self.source()));
            return;
        }
        if let Some(normalized) =
            normalize_scalar_assignment_unquoted_continuations(assignment, self.source())
        {
            self.write_text(&normalized);
            return;
        }

        let source_map = self.source_map().clone();
        let mut scratch = self.take_scratch_buffer();
        {
            let facts = self.facts();
            render_assignment_with_facts_to_buf(
                assignment,
                self.source(),
                self.options(),
                &source_map,
                facts,
                &mut scratch,
            );
        }
        if assignment_contains_command_heredoc(assignment)
            && rendered_shell_text_has_heredoc_tail(&scratch)
        {
            self.write_rendered_shell_text_preserving_heredoc_tails(&scratch);
        } else if scratch.contains('\n')
            && assignment_value_is_quoted_formattable_command_substitution_only(
                assignment,
                self.source(),
            )
        {
            self.write_text_preserving_current_line_indent(&scratch);
        } else if scratch.contains('\n')
            && assignment_source_has_command_substitution(assignment, self.source())
        {
            if compound_assignment_is_single_case_command_substitution(assignment) {
                self.write_text_preserving_current_line_indent(&scratch);
            } else if assignment_has_multiline_literal_source(assignment, self.source()) {
                if assignment_value_is_quoted_command_substitution_only(assignment) {
                    self.write_command_substitution_assignment_text(&scratch);
                } else if assignment_source_has_leading_pipe_continuation(assignment, self.source())
                {
                    self.write_text_preserving_current_line_indent(&scratch);
                } else {
                    let continuation_indent =
                        self.indent_prefix_for_level(self.indent_level.saturating_add(1));
                    let normalized = normalize_literal_assignment_command_substitution_pipelines(
                        &scratch,
                        &continuation_indent,
                    );
                    self.write_rendered_shell_text(&normalized);
                }
            } else {
                self.write_text_preserving_current_line_indent(&scratch);
            }
        } else if assignment_has_multiline_literal_source(assignment, self.source()) {
            self.write_rendered_shell_text(&scratch);
        } else {
            self.write_text(&scratch);
        }
        self.restore_scratch_buffer(scratch);
    }

    fn write_assignment_head(&mut self, assignment: &Assignment) {
        self.write_rendered(|scratch, source, _| {
            render_assignment_head_to_buf(assignment, source, scratch);
        });
    }

    fn write_rendered_name_if_nonempty(
        &mut self,
        rendered_name: &str,
        previous_end: Option<usize>,
        name_span: Span,
    ) -> Option<usize> {
        if rendered_name.is_empty() {
            previous_end
        } else {
            self.write_command_gap(previous_end, name_span.start.offset);
            self.write_text(rendered_name);
            Some(name_span.end.offset)
        }
    }

    fn write_comment(&mut self, comment: &SourceComment<'_>) {
        self.write_text(comment.text());
    }

    fn emit_leading_comments(&mut self, comments: &[SourceComment<'_>], next_line: usize) {
        for (index, comment) in comments.iter().enumerate() {
            self.write_comment(comment);
            let target_line = comments
                .get(index + 1)
                .map(SourceComment::line)
                .unwrap_or(next_line);
            self.write_line_breaks(line_gap_break_count(comment.line(), target_line));
        }
    }

    fn emit_pipeline_leading_comments_after_operator(
        &mut self,
        comments: &[SourceComment<'_>],
        next_line: usize,
        operator_end: usize,
    ) {
        let preserve_blank_before_comments = comments.first().is_some_and(|comment| {
            gap_has_blank_line(self.source(), operator_end, comment.span().start.offset)
        });
        if preserve_blank_before_comments {
            self.newline();
        }

        for (index, comment) in comments.iter().enumerate() {
            self.write_comment(comment);
            let target_line = comments
                .get(index + 1)
                .map(SourceComment::line)
                .unwrap_or(next_line);
            let breaks = if preserve_blank_before_comments && index + 1 == comments.len() {
                1
            } else {
                line_gap_break_count(comment.line(), target_line)
            };
            self.write_line_breaks(breaks);
        }
    }

    fn emit_trailing_comments_for_stmt(&mut self, comments: &[SourceComment<'_>]) {
        for comment in comments {
            let current_code_column = self.column.saturating_sub(self.line_indent_column);
            let padding = trailing_comment_padding(self.source(), comment, current_code_column);
            for _ in 0..padding {
                self.write_space();
            }
            self.write_comment(comment);
        }
    }

    fn emit_dangling_comments(&mut self, comments: &[SourceComment<'_>]) {
        self.emit_dangling_comments_after(comments, None);
    }

    fn emit_dangling_comments_after(
        &mut self,
        comments: &[SourceComment<'_>],
        previous_line: Option<usize>,
    ) {
        for (index, comment) in comments.iter().enumerate() {
            if index == 0 {
                if let Some(previous_line) = previous_line {
                    self.write_line_breaks(line_gap_break_count(previous_line, comment.line()));
                } else {
                    self.newline();
                }
            }
            self.maybe_preserve_dangling_comment_outdent(comment);
            self.write_comment(comment);
            if let Some(next) = comments.get(index + 1) {
                self.write_line_breaks(line_gap_break_count(comment.line(), next.line()));
            }
        }
    }

    fn maybe_preserve_dangling_comment_outdent(&mut self, comment: &SourceComment<'_>) {
        if !self.line_start {
            return;
        }
        if comment_precedes_close_keyword_at_same_indent(self.source(), comment) {
            let close_indent_column =
                self.indent_column_for_level(self.indent_level.saturating_sub(1));
            if close_indent_column == 0 {
                self.line_indent_column = 0;
                self.line_start = false;
            } else {
                self.write_indent_to_column(close_indent_column);
            }
        }
    }

    fn format_stmt_sequence(
        &mut self,
        statements: &StmtSeq,
        upper_bound: Option<usize>,
    ) -> Result<()> {
        self.format_stmt_sequence_with_leading_filter(statements, upper_bound, None)
    }

    fn format_stmt_sequence_with_leading_filter(
        &mut self,
        statements: &StmtSeq,
        upper_bound: Option<usize>,
        first_leading_min_offset: Option<usize>,
    ) -> Result<()> {
        let source = self.source();
        let compact_layout = self.options().compact_layout();
        let minify = self.options().minify();
        let attachments = (!minify).then(|| self.facts().sequence(statements, upper_bound).clone());
        let compact = compact_layout
            && attachments
                .as_ref()
                .is_none_or(|sequence| !sequence.has_comments());

        if statements.is_empty() {
            if let Some(attachment) = attachments.as_ref() {
                let comments = attachment.dangling();
                if let Some((first, rest)) = comments.split_first() {
                    self.write_comment(first);
                    let mut previous = first;
                    for comment in rest {
                        self.write_line_breaks(line_gap_break_count(
                            previous.line(),
                            comment.line(),
                        ));
                        self.write_comment(comment);
                        previous = comment;
                    }
                }
            }
            return Ok(());
        }

        if first_leading_min_offset.is_none()
            && attachments
                .as_ref()
                .is_some_and(|value| value.is_ambiguous())
            && let Some(span) = sequence_verbatim_span(statements, self.source_map())
        {
            if let Some(attachment) = attachments.as_ref()
                && let Some(first) = statements.first()
            {
                let leading = attachment
                    .leading_for(0)
                    .iter()
                    .copied()
                    .filter(|comment| comment.span().end.offset <= span.start.offset)
                    .collect::<Vec<_>>();
                self.emit_leading_comments(
                    &leading,
                    self.facts().stmt(first).render_span().start.line,
                );
            }
            self.write_verbatim(span.slice(source));
            if let Some(attachment) = attachments.as_ref() {
                self.emit_dangling_comments(attachment.dangling());
            }
            return Ok(());
        }

        for (index, stmt) in statements.iter().enumerate() {
            if let Some(attachment) = attachments.as_ref() {
                let next_line =
                    stmt_render_start_line(stmt, self.source(), self.source_map(), self.options());
                if index == 0
                    && let Some(min_offset) = first_leading_min_offset
                {
                    let leading = attachment
                        .leading_for(index)
                        .iter()
                        .copied()
                        .filter(|comment| comment.span().start.offset >= min_offset)
                        .collect::<Vec<_>>();
                    self.emit_leading_comments(&leading, next_line);
                } else {
                    self.emit_leading_comments(attachment.leading_for(index), next_line);
                }
            }

            self.format_stmt(stmt)?;

            if let Some(attachment) = attachments.as_ref() {
                self.emit_trailing_comments_for_stmt(attachment.trailing_for(index));
            }

            if index + 1 < statements.len() {
                if matches!(stmt.terminator, Some(StmtTerminator::Background(_))) {
                    if stmt_is_redirect_only(&statements[index + 1], source) {
                        self.write_line_breaks(1);
                    } else if self.facts().background_has_explicit_line_break(stmt) {
                        let current_end = stmt_rendered_end_line_after_format(
                            stmt,
                            source,
                            self.source_map(),
                            self.facts().stmt(stmt).rendered_end_line(),
                        );
                        let next_start = attachments
                            .as_ref()
                            .map(|attachment| attachment.first_rendered_line_for(index + 1))
                            .unwrap_or(stmt_render_start_line(
                                &statements[index + 1],
                                self.source(),
                                self.source_map(),
                                self.options(),
                            ));
                        self.write_line_breaks(line_gap_break_count(current_end, next_start));
                    } else {
                        self.write_space();
                    }
                } else if compact {
                    self.write_text("; ");
                } else {
                    let current_end = stmt_rendered_end_line_after_format(
                        stmt,
                        source,
                        self.source_map(),
                        self.facts().stmt(stmt).rendered_end_line(),
                    );
                    let next_start = attachments
                        .as_ref()
                        .map(|attachment| attachment.first_rendered_line_for(index + 1))
                        .unwrap_or(stmt_render_start_line(
                            &statements[index + 1],
                            self.source(),
                            self.source_map(),
                            self.options(),
                        ));
                    self.write_line_breaks(line_gap_break_count(current_end, next_start));
                }
            }
        }

        self.flush_pending_heredocs();

        if let Some(attachment) = attachments.as_ref() {
            let previous_line = statements.last().map(|stmt| {
                stmt_rendered_end_line_after_format(
                    stmt,
                    source,
                    self.source_map(),
                    self.facts().stmt(stmt).rendered_end_line(),
                )
            });
            self.emit_dangling_comments_after(attachment.dangling(), previous_line);
        }
        Ok(())
    }

    fn format_stmt(&mut self, stmt: &Stmt) -> Result<()> {
        let source = self.source();
        let stmt_facts = self.facts().stmt(stmt);
        if stmt_facts.preserve_verbatim() {
            self.write_verbatim(stmt_facts.render_span().slice(source));
            return Ok(());
        }

        if stmt.negated {
            self.write_text("! ");
        }

        let command_span = command_format_span(&stmt.command);
        let emit_redirects_first = !stmt.redirects.is_empty()
            && command_span != Span::new()
            && stmt
                .redirects
                .iter()
                .all(|redirect| redirect.span.start.offset < command_span.start.offset);

        if emit_redirects_first {
            self.format_redirect_list(&stmt.redirects);
            if command_span != Span::new() {
                self.write_space();
            }
        }

        let redirects_formatted_with_command = matches!(&stmt.command, Command::Simple(_))
            && !stmt.redirects.is_empty()
            && !emit_redirects_first;

        match &stmt.command {
            Command::Simple(command) if redirects_formatted_with_command => {
                self.format_simple_command_with_redirects(command, &stmt.redirects);
            }
            Command::Compound(CompoundCommand::BraceGroup(commands)) => {
                self.format_brace_group(commands, Some(stmt_span(stmt).end.offset))?;
            }
            Command::Compound(CompoundCommand::Subshell(commands)) => {
                self.format_subshell(commands, Some(stmt_span(stmt).end.offset))?;
            }
            _ => self.format_command(&stmt.command)?,
        }

        if !stmt.redirects.is_empty() && !emit_redirects_first && !redirects_formatted_with_command
        {
            if redirect_list_needs_leading_space(command_span, &stmt.redirects, source) {
                self.write_space();
            }
            self.format_redirect_list(&stmt.redirects);
        }

        self.queue_heredocs(&stmt.redirects);

        match stmt.terminator {
            Some(StmtTerminator::Background(operator)) => {
                self.write_space();
                self.write_text(render_background_operator(operator));
            }
            Some(StmtTerminator::Semicolon)
                if stmt_semicolon_terminator_starts_on_continuation_line(stmt, source) =>
            {
                self.line_continuation();
                self.write_indent_units(1);
                self.write_text(";");
            }
            _ => {}
        }

        Ok(())
    }

    fn format_command(&mut self, command: &Command) -> Result<()> {
        match command {
            Command::Simple(command) => self.format_simple_command(command),
            Command::Builtin(command) => self.format_builtin_command(command),
            Command::Decl(command) => self.format_decl_clause(command),
            Command::Binary(command) => self.format_binary_command(command),
            Command::Compound(compound) => self.format_compound_command(compound),
            Command::Function(function) => self.format_function(function),
            Command::AnonymousFunction(function) => self.format_anonymous_function(function),
        }
    }

    fn format_compound_command(&mut self, command: &CompoundCommand) -> Result<()> {
        match command {
            CompoundCommand::If(command) => self.format_if(command),
            CompoundCommand::For(command) => self.format_for(command),
            CompoundCommand::Repeat(command) => self.format_repeat(command),
            CompoundCommand::Foreach(command) => self.format_foreach(command),
            CompoundCommand::ArithmeticFor(command) => self.format_arithmetic_for(command),
            CompoundCommand::While(command) => self.format_while(command),
            CompoundCommand::Until(command) => self.format_until(command),
            CompoundCommand::Case(command) => self.format_case(command),
            CompoundCommand::Select(command) => self.format_select(command),
            CompoundCommand::Subshell(commands) => self.format_subshell(commands, None),
            CompoundCommand::BraceGroup(commands) => self.format_brace_group(commands, None),
            CompoundCommand::Arithmetic(command) => self.format_arithmetic(command),
            CompoundCommand::Time(command) => self.format_time(command),
            CompoundCommand::Conditional(command) => self.format_conditional(command),
            CompoundCommand::Coproc(command) => self.format_coproc(command),
            CompoundCommand::Always(command) => self.format_always(command),
        }
    }

    fn format_simple_command(&mut self, command: &SimpleCommand) -> Result<()> {
        let source = self.source();
        let source_map = self.source_map().clone();
        let mut rendered_name = self.take_scratch_buffer();
        {
            let facts = self.facts();
            render_word_syntax_with_facts_to_buf(
                &command.name,
                source,
                self.options(),
                &source_map,
                facts,
                &mut rendered_name,
            );
        }
        if command.args.is_empty()
            && command.assignments.len() == 1
            && rendered_name.is_empty()
            && multiline_compound_assignment_lines(&command.assignments[0], source).is_some()
        {
            self.restore_scratch_buffer(rendered_name);
            return self.format_standalone_multiline_compound_assignment(&command.assignments[0]);
        }

        let mut previous_end = None;
        let keep_assignment_continuations_flush_left =
            command.assignments.first().is_some_and(|assignment| {
                assignment_source_has_command_substitution(assignment, source)
            });
        for assignment in &command.assignments {
            if keep_assignment_continuations_flush_left
                && previous_end.is_some_and(|previous_end| {
                    has_newline_between_offsets(source, previous_end, assignment.span.start.offset)
                })
            {
                self.line_continuation();
            } else {
                self.write_command_gap(previous_end, assignment.span.start.offset);
            }
            self.write_assignment(assignment);
            previous_end = Some(assignment.span.end.offset);
        }
        previous_end =
            self.write_rendered_name_if_nonempty(&rendered_name, previous_end, command.name.span);
        self.restore_scratch_buffer(rendered_name);
        for argument in &command.args {
            self.write_command_gap(previous_end, argument.span.start.offset);
            self.write_word(argument);
            previous_end = Some(word_gap_end_before_trailing_continuation(
                argument,
                self.source(),
            ));
        }
        Ok(())
    }

    fn format_simple_command_with_redirects(
        &mut self,
        command: &SimpleCommand,
        redirects: &[Redirect],
    ) {
        let source = self.source();
        let source_map = self.source_map().clone();
        let mut rendered_name = self.take_scratch_buffer();
        {
            let facts = self.facts();
            render_word_syntax_with_facts_to_buf(
                &command.name,
                source,
                self.options(),
                &source_map,
                facts,
                &mut rendered_name,
            );
        }

        let mut parts = Vec::with_capacity(
            command.assignments.len()
                + usize::from(!rendered_name.is_empty())
                + command.args.len()
                + redirects.len(),
        );
        parts.extend(
            command
                .assignments
                .iter()
                .map(SimpleCommandPart::Assignment),
        );
        if !rendered_name.is_empty() {
            parts.push(SimpleCommandPart::Name);
        }
        parts.extend(command.args.iter().map(SimpleCommandPart::Argument));
        parts.extend(redirects.iter().map(SimpleCommandPart::Redirect));
        parts.sort_by_key(|part| part.start_offset(command));
        move_interspersed_redirects_after_arguments(&mut parts);

        let keep_assignment_continuations_flush_left =
            command.assignments.first().is_some_and(|assignment| {
                assignment_source_has_command_substitution(assignment, source)
            });
        let mut previous_part = None;
        let mut previous_end = None;
        let mut part_index = 0;
        while part_index < parts.len() {
            let part = parts[part_index];
            if matches!(
                (previous_part, part),
                (
                    Some(SimpleCommandPart::Assignment(_)),
                    SimpleCommandPart::Assignment(_)
                ) if keep_assignment_continuations_flush_left
            ) && previous_end.is_some_and(|previous_end| {
                has_newline_between_offsets(source, previous_end, part.start_offset(command))
            }) {
                self.line_continuation();
            } else if let SimpleCommandPart::Redirect(redirect) = &part {
                self.write_redirect_gap(previous_part, previous_end, redirect, command);
            } else {
                self.write_command_gap(previous_end, part.start_offset(command));
            }
            let end_offset = part.end_offset(command);
            match part {
                SimpleCommandPart::Assignment(assignment) => self.write_assignment(assignment),
                SimpleCommandPart::Name => self.write_text(&rendered_name),
                SimpleCommandPart::Argument(argument) => self.write_word(argument),
                SimpleCommandPart::Redirect(redirect) => {
                    if let Some(SimpleCommandPart::Redirect(next)) =
                        parts.get(part_index + 1).copied()
                        && append_both_redirect_pair_matches_source(redirect, next, source)
                    {
                        self.format_append_both_redirect(redirect);
                        part_index += 1;
                    } else {
                        self.format_redirect(redirect);
                    }
                }
            }
            previous_part = Some(part);
            previous_end = Some(end_offset);
            part_index += 1;
        }
        self.restore_scratch_buffer(rendered_name);
    }

    fn write_redirect_gap(
        &mut self,
        previous_part: Option<SimpleCommandPart<'_>>,
        previous_end: Option<usize>,
        redirect: &Redirect,
        command: &SimpleCommand,
    ) {
        let Some(previous_end) = previous_end else {
            return;
        };
        if previous_end == redirect.span.start.offset {
            if redirect_has_adjacent_numeric_fd_prefix(
                previous_part,
                redirect,
                command,
                self.source(),
            ) {
                return;
            }
            if !redirect_is_attached_process_substitution(Span::new(), redirect, self.source()) {
                self.write_space();
            }
            return;
        }
        self.write_command_gap(Some(previous_end), redirect.span.start.offset);
    }

    fn format_builtin_command(&mut self, command: &BuiltinCommand) -> Result<()> {
        match command {
            BuiltinCommand::Break(command) => self.format_builtin_like(
                "break",
                command.span.start,
                &command.assignments,
                command.depth.as_ref(),
                &command.extra_args,
            ),
            BuiltinCommand::Continue(command) => self.format_builtin_like(
                "continue",
                command.span.start,
                &command.assignments,
                command.depth.as_ref(),
                &command.extra_args,
            ),
            BuiltinCommand::Return(command) => self.format_builtin_like(
                "return",
                command.span.start,
                &command.assignments,
                command.code.as_ref(),
                &command.extra_args,
            ),
            BuiltinCommand::Exit(command) => self.format_builtin_like(
                "exit",
                command.span.start,
                &command.assignments,
                command.code.as_ref(),
                &command.extra_args,
            ),
        }
    }

    fn format_builtin_like(
        &mut self,
        name: &str,
        start: shuck_ast::Position,
        assignments: &[shuck_ast::Assignment],
        primary: Option<&Word>,
        extra_args: &[Word],
    ) -> Result<()> {
        let mut previous_end = None;
        for assignment in assignments {
            self.write_command_gap(previous_end, assignment.span.start.offset);
            self.write_assignment(assignment);
            previous_end = Some(assignment.span.end.offset);
        }
        let name_span = Span::from_positions(start, start.advanced_by(name));
        self.write_command_gap(previous_end, name_span.start.offset);
        self.write_text(name);
        previous_end = Some(name_span.end.offset);
        if let Some(primary) = primary {
            self.write_command_gap(previous_end, primary.span.start.offset);
            self.write_word(primary);
            previous_end = Some(primary.span.end.offset);
        }
        for argument in extra_args {
            self.write_command_gap(previous_end, argument.span.start.offset);
            self.write_word(argument);
            previous_end = Some(argument.span.end.offset);
        }
        Ok(())
    }

    fn format_decl_clause(&mut self, command: &DeclClause) -> Result<()> {
        let mut previous_end = None;
        for assignment in &command.assignments {
            self.write_command_gap(previous_end, assignment.span.start.offset);
            self.write_assignment(assignment);
            previous_end = Some(assignment.span.end.offset);
        }
        self.write_command_gap(previous_end, command.variant_span.start.offset);
        self.write_text(command.variant.as_ref());
        previous_end = Some(command.variant_span.end.offset);
        for operand in &command.operands {
            let span = decl_operand_span(operand);
            self.write_command_gap(previous_end, span.start.offset);
            self.write_decl_operand(operand);
            previous_end = Some(span.end.offset);
        }
        Ok(())
    }

    fn write_command_gap(&mut self, previous_end: Option<usize>, next_start: usize) {
        let Some(previous_end) = previous_end else {
            return;
        };
        if previous_end == next_start {
            return;
        }
        if self
            .source()
            .get(previous_end..next_start)
            .is_some_and(|between| between.contains('\n'))
        {
            self.line_continuation();
            self.write_indent_units(1);
        } else {
            self.write_space();
        }
    }

    fn write_word_list_preserving_breaks(&mut self, words: &[Word]) {
        self.write_word_list_preserving_breaks_after(words, None);
    }

    fn write_word_list_preserving_breaks_after(
        &mut self,
        words: &[Word],
        first_previous_end: Option<usize>,
    ) {
        let mut previous_end = first_previous_end;
        for word in words {
            if let Some(previous_end) = previous_end {
                self.write_command_gap(Some(previous_end), word.span.start.offset);
            } else {
                self.write_space();
            }
            self.write_word(word);
            previous_end = Some(word_gap_end_before_trailing_continuation(
                word,
                self.source(),
            ));
        }
    }

    fn format_do_done_body(
        &mut self,
        body: &StmtSeq,
        enclosing_span: Span,
        close_span: Option<Span>,
        close: &'static str,
    ) -> Result<()> {
        let body_upper_bound = close_span
            .map(|span| span.start.offset)
            .unwrap_or(enclosing_span.end.offset);
        let has_open_suffix = self
            .facts()
            .sequence(body, Some(body_upper_bound))
            .group_open_suffix_span()
            .is_some();
        if !has_open_suffix
            && self.can_inline_body_with_upper_bound(body, enclosing_span, Some(body_upper_bound))
        {
            self.write_text("; do ");
            self.format_inline_stmts(body)?;
            self.write_text("; ");
            self.write_text(close);
            self.write_close_suffix_after_span(close_span);
            return Ok(());
        }

        if !has_open_suffix && self.body_starts_with_inline_do_brace_group(body) {
            self.write_text("; do ");
            self.format_stmt(&body[0])?;
            self.write_text(self.inline_do_brace_group_done_separator(body, enclosing_span));
            self.write_text(close);
            self.write_close_suffix_after_span(close_span);
            return Ok(());
        }

        if !has_open_suffix && self.body_starts_with_inline_do_if(body) {
            self.write_text("; do ");
            self.format_stmt(&body[0])?;
            self.write_text("; ");
            self.write_text(close);
            self.write_close_suffix_after_span(close_span);
            return Ok(());
        }

        self.write_text("; do");
        self.write_sequence_open_suffix(body, Some(body_upper_bound));
        let preserve_open_blank = body_has_blank_line_after_keyword(
            self.source(),
            self.source_map(),
            enclosing_span.start.offset,
            "do",
            body,
        );
        self.format_body_with_upper_bound_and_open_blank(
            body,
            Some(body_upper_bound),
            preserve_open_blank,
        )?;
        self.write_unmodeled_branch_background_terminator(body, body_upper_bound);
        if close_span.is_some_and(|span| {
            source_has_blank_line_immediately_before_offset(self.source(), span.start.offset)
        }) || (close_span.is_none()
            && source_has_blank_line_before_last_keyword(
                self.source(),
                self.source_map(),
                enclosing_span,
                close,
            ))
        {
            self.newline();
        }
        self.finish_block_with_close_suffix(close, close_span);
        Ok(())
    }

    fn format_split_do_done_body(
        &mut self,
        body: &StmtSeq,
        enclosing_span: Span,
        close_span: Option<Span>,
        close: &'static str,
    ) -> Result<()> {
        let body_upper_bound = close_span
            .map(|span| span.start.offset)
            .unwrap_or(enclosing_span.end.offset);

        self.write_text("do");
        self.write_sequence_open_suffix(body, Some(body_upper_bound));
        let preserve_open_blank = body_has_blank_line_after_keyword(
            self.source(),
            self.source_map(),
            enclosing_span.start.offset,
            "do",
            body,
        );
        self.format_body_with_upper_bound_and_open_blank(
            body,
            Some(body_upper_bound),
            preserve_open_blank,
        )?;
        self.write_unmodeled_branch_background_terminator(body, body_upper_bound);
        if close_span.is_some_and(|span| {
            source_has_blank_line_immediately_before_offset(self.source(), span.start.offset)
        }) || (close_span.is_none()
            && source_has_blank_line_before_last_keyword(
                self.source(),
                self.source_map(),
                enclosing_span,
                close,
            ))
        {
            self.newline();
        }
        self.finish_block_with_close_suffix(close, close_span);
        Ok(())
    }

    fn body_starts_with_inline_do_brace_group(&self, body: &StmtSeq) -> bool {
        let [stmt] = body.as_slice() else {
            return false;
        };
        if stmt.negated || !stmt.redirects.is_empty() || stmt.terminator.is_some() {
            return false;
        }
        let Command::Compound(CompoundCommand::BraceGroup(commands)) = &stmt.command else {
            return false;
        };
        let Some(group_span) =
            group_attachment_span(commands.as_slice(), self.source_map(), '{', '}')
        else {
            return false;
        };
        let source = self.source();
        let line_start = source[..group_span.start.offset]
            .rfind('\n')
            .map_or(0, |offset| offset.saturating_add(1));
        source[line_start..group_span.start.offset]
            .trim_end_matches([' ', '\t', '\r'])
            .ends_with("do")
    }

    fn body_starts_with_inline_do_if(&self, body: &StmtSeq) -> bool {
        let [stmt] = body.as_slice() else {
            return false;
        };
        if stmt.negated || !stmt.redirects.is_empty() || stmt.terminator.is_some() {
            return false;
        }
        let Command::Compound(CompoundCommand::If(command)) = &stmt.command else {
            return false;
        };
        if !matches!(command.syntax, IfSyntax::ThenFi { .. }) {
            return false;
        }
        let source = self.source();
        let line_start = source[..command.span.start.offset]
            .rfind('\n')
            .map_or(0, |offset| offset.saturating_add(1));
        source[line_start..command.span.start.offset]
            .trim_end_matches([' ', '\t', '\r'])
            .ends_with("do")
    }

    fn inline_do_brace_group_done_separator(
        &self,
        body: &StmtSeq,
        enclosing_span: Span,
    ) -> &'static str {
        let [stmt] = body.as_slice() else {
            return "; ";
        };
        let Command::Compound(CompoundCommand::BraceGroup(commands)) = &stmt.command else {
            return "; ";
        };
        let Some(group_span) =
            group_attachment_span(commands.as_slice(), self.source_map(), '{', '}')
        else {
            return "; ";
        };
        let source = self.source();
        let between = source
            .get(group_span.end.offset..enclosing_span.end.offset)
            .unwrap_or_default()
            .trim_start_matches([' ', '\t', '\r']);
        if between.starts_with(';') {
            return "; ";
        }
        if brace_group_last_stmt_allows_done_without_semicolon(commands) {
            " "
        } else {
            "; "
        }
    }

    fn write_decl_operand(&mut self, operand: &DeclOperand) {
        match operand {
            DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => self.write_word(word),
            DeclOperand::Name(name) => self.write_var_ref(name),
            DeclOperand::Assignment(assignment)
                if multiline_compound_assignment_lines(assignment, self.source()).is_some() =>
            {
                self.format_inline_multiline_compound_assignment(assignment);
            }
            DeclOperand::Assignment(assignment) => self.write_assignment(assignment),
        }
    }

    fn format_inline_multiline_compound_assignment(&mut self, assignment: &Assignment) {
        if self.multiline_compound_assignment_needs_structural_elements(assignment) {
            self.format_structural_multiline_compound_assignment(assignment);
            return;
        }
        if assignment_has_multiline_literal_source(assignment, self.source())
            && !Self::compound_assignment_source_has_line_continuations(
                assignment.span.slice(self.source()),
            )
        {
            self.write_multiline_compound_literal_assignment(assignment);
            return;
        }

        let Some(layout) = multiline_compound_assignment_layout(assignment, self.source()) else {
            self.write_assignment(assignment);
            return;
        };

        self.write_assignment_head(assignment);
        self.write_text("(");
        let body_start = if layout.open_inline {
            if let Some(first) = layout.lines.first() {
                self.write_text(first);
            }
            1
        } else {
            0
        };

        for (index, line) in layout.lines[body_start..].iter().enumerate() {
            self.newline();
            let closes_inline_assignment =
                layout.close_inline && body_start + index + 1 == layout.lines.len();
            self.write_indent_units(multiline_compound_assignment_line_extra_indent(
                line,
                closes_inline_assignment,
            ));
            self.write_text(line);
        }
        if layout.close_inline {
            self.write_text(")");
        } else {
            self.newline();
            self.write_text(")");
        }
    }

    fn multiline_compound_assignment_needs_structural_elements(
        &self,
        assignment: &Assignment,
    ) -> bool {
        let AssignmentValue::Compound(array) = &assignment.value else {
            return false;
        };
        let raw = assignment.span.slice(self.source());
        if !raw.contains("$(")
            || !raw.contains(';')
            || raw.contains('#')
            || raw.contains("\n\n")
            || assignment_has_multiline_literal_source(assignment, self.source())
        {
            return false;
        }

        array.elements.iter().any(|element| match element {
            ArrayElem::Sequential(word)
            | ArrayElem::Keyed { value: word, .. }
            | ArrayElem::KeyedAppend { value: word, .. } => {
                word_contains_command_substitution(word)
            }
        })
    }

    fn format_structural_multiline_compound_assignment(&mut self, assignment: &Assignment) {
        let AssignmentValue::Compound(array) = &assignment.value else {
            self.write_assignment(assignment);
            return;
        };

        self.write_assignment_head(assignment);
        self.write_text("(");
        for element in &array.elements {
            self.newline();
            self.write_indent_units(1);
            self.write_array_element(element);
        }
        self.newline();
        self.write_text(")");
    }

    fn write_array_element(&mut self, element: &ArrayElem) {
        match element {
            ArrayElem::Sequential(word) => self.write_word(word),
            ArrayElem::Keyed { key, value } => self.write_keyed_array_element(key, value, "="),
            ArrayElem::KeyedAppend { key, value } => {
                self.write_keyed_array_element(key, value, "+=");
            }
        }
    }

    fn write_keyed_array_element(&mut self, key: &shuck_ast::Subscript, value: &Word, op: &str) {
        self.write_text("[");
        self.write_rendered(|scratch, source, _| {
            render_subscript_to_buf(key, source, scratch);
        });
        self.write_text("]");
        self.write_text(op);
        self.write_word(value);
    }

    fn write_standalone_multiline_compound_assignment_layout(
        &mut self,
        layout: &crate::command::MultilineCompoundAssignmentLayout,
    ) {
        let body_start = if layout.open_inline {
            if let Some(first) = layout.lines.first() {
                self.write_text(first);
            }
            1
        } else {
            0
        };

        if body_start < layout.lines.len() {
            self.newline();
            for (index, line) in layout.lines[body_start..].iter().enumerate() {
                if index > 0 {
                    self.newline();
                }
                let closes_inline_assignment =
                    layout.close_inline && body_start + index + 1 == layout.lines.len();
                self.with_extra_prefix_indent(
                    multiline_compound_assignment_line_extra_indent(line, closes_inline_assignment),
                    |formatter| formatter.write_text(line),
                );
            }
        }

        if layout.close_inline {
            self.write_text(")");
        } else {
            self.newline();
            self.write_text(")");
        }
    }

    fn format_binary_command(&mut self, command: &BinaryCommand) -> Result<()> {
        match command.op {
            BinaryOp::Pipe | BinaryOp::PipeAll => self.format_pipeline(command),
            BinaryOp::And | BinaryOp::Or => self.format_command_list(command),
        }
    }

    fn format_pipeline(&mut self, pipeline: &BinaryCommand) -> Result<()> {
        let mut statements = Vec::new();
        let mut operators = Vec::new();
        collect_pipeline(pipeline, &mut statements, &mut operators);

        let mut operator_breaks = pipeline_operator_breaks(
            &statements,
            &operators,
            self.source(),
            self.source_map(),
            self.options(),
        );
        if self.facts().pipeline_has_explicit_line_break(pipeline)
            && !operator_breaks.iter().any(|broken| *broken)
        {
            operator_breaks.fill(true);
        }
        let operator_next_line = self.options().binary_next_line();

        for (index, stmt) in statements.iter().enumerate() {
            if index == 0 {
                self.format_pipeline_stmt(stmt)?;
                continue;
            }
            if index > 0 {
                let (operator, operator_span) = operators
                    .get(index - 1)
                    .map(|(operator, span)| (binary_operator(operator), *span))
                    .unwrap_or(("|", stmt.span));
                let break_here = operator_breaks.get(index - 1).copied().unwrap_or(false);
                if break_here && operator_next_line {
                    self.line_continuation();
                    self.with_extra_prefix_indent(
                        self.pipeline_continuation_indent,
                        |formatter| {
                            formatter.write_text(operator);
                            formatter.write_space();
                            formatter.format_pipeline_stmt_after_operator(stmt, operator_span)
                        },
                    )?;
                    continue;
                }
                if break_here {
                    self.write_space();
                    self.write_text(operator);
                    self.newline();
                    self.emit_pipeline_interstitial_comments(stmt, operator_span);
                    self.with_extra_prefix_indent(
                        self.pipeline_continuation_indent,
                        |formatter| {
                            formatter.format_pipeline_stmt_after_operator(stmt, operator_span)
                        },
                    )?;
                    continue;
                }
                self.write_space();
                self.write_text(operator);
                self.write_space();
            }
            self.format_stmt(stmt)?;
        }

        Ok(())
    }

    fn emit_pipeline_interstitial_comments(&mut self, stmt: &Stmt, operator_span: Span) {
        if stmt.leading_comments.iter().any(|comment| {
            self.source_map()
                .source_comment(*comment)
                .is_some_and(|comment| {
                    !comment.inline() && comment.span().start.offset >= operator_span.end.offset
                })
        }) {
            return;
        }
        let command_start = pipeline_interstitial_comment_end(stmt, self.source_map());
        if command_start <= operator_span.end.offset {
            return;
        }
        let comments = self.own_line_comments_in_region(operator_span.end.offset, command_start);
        for comment in comments {
            self.with_extra_prefix_indent(self.pipeline_continuation_indent, |formatter| {
                formatter.write_text(&comment.text);
            });
            self.newline();
        }
    }

    fn own_line_comments_in_region(&self, start: usize, end: usize) -> Vec<BranchPrefixComment> {
        own_line_comments_in_region(self.source(), start, end)
            .into_iter()
            .filter(|comment| !self.facts().offset_is_in_heredoc_body(comment.offset))
            .collect()
    }

    fn format_pipeline_stmt(&mut self, stmt: &Stmt) -> Result<()> {
        self.format_pipeline_stmt_with_leading_comment_start(stmt, None)
    }

    fn format_pipeline_stmt_after_operator(
        &mut self,
        stmt: &Stmt,
        operator_span: Span,
    ) -> Result<()> {
        self.format_pipeline_stmt_with_leading_comment_start(stmt, Some(operator_span.end.offset))
    }

    fn format_pipeline_stmt_with_leading_comment_start(
        &mut self,
        stmt: &Stmt,
        min_comment_start: Option<usize>,
    ) -> Result<()> {
        let statement_start =
            stmt_attachment_span(stmt, self.source(), self.source_map(), self.options())
                .start
                .offset;
        let next_line =
            stmt_render_start_line(stmt, self.source(), self.source_map(), self.options());
        let leading = stmt
            .leading_comments
            .iter()
            .filter_map(|comment| self.source_map().source_comment(*comment))
            .filter(|comment| {
                !comment.inline()
                    && comment.span().end.offset <= statement_start
                    && min_comment_start
                        .is_none_or(|min_start| comment.span().start.offset >= min_start)
            })
            .collect::<Vec<_>>();
        if let Some(operator_end) = min_comment_start {
            self.emit_pipeline_leading_comments_after_operator(&leading, next_line, operator_end);
        } else {
            self.emit_leading_comments(&leading, next_line);
        }
        self.format_stmt(stmt)
    }

    fn format_command_list(&mut self, list: &BinaryCommand) -> Result<()> {
        let mut rest = Vec::new();
        let first = collect_command_list_first(list, &mut rest);
        self.format_stmt(first)?;
        for item in &rest {
            self.format_list_item(item)?;
        }
        Ok(())
    }

    fn format_list_item(&mut self, item: &BinaryListItem<'_>) -> Result<()> {
        if self
            .facts()
            .list_item_has_explicit_line_break(item.operator_span)
        {
            self.write_text(list_item_multiline_separator(item.operator));
            self.newline();
            self.with_indent(|formatter| {
                let emitted_interstitial_comments = formatter
                    .emit_command_list_interstitial_comments(item.stmt, item.operator_span);
                if stmt_is_pipeline(item.stmt) {
                    formatter.with_pipeline_continuation_indent(0, |formatter| {
                        formatter.with_group_body_leading_filter(
                            emitted_interstitial_comments,
                            |formatter| formatter.format_stmt(item.stmt),
                        )
                    })
                } else {
                    formatter.with_group_body_leading_filter(
                        emitted_interstitial_comments,
                        |formatter| formatter.format_stmt(item.stmt),
                    )
                }
            })?;
            return Ok(());
        }

        self.write_text(list_item_inline_separator(item.operator));
        self.format_stmt(item.stmt)
    }

    fn emit_command_list_interstitial_comments(
        &mut self,
        stmt: &Stmt,
        operator_span: Span,
    ) -> bool {
        let command_start = command_format_span(&stmt.command).start.offset;
        if command_start <= operator_span.end.offset {
            return false;
        }
        let comments = self.own_line_comments_in_region(operator_span.end.offset, command_start);
        let emitted = !comments.is_empty();
        for comment in comments {
            self.write_text(&comment.text);
            self.newline();
        }
        emitted
    }

    fn format_if(&mut self, command: &IfCommand) -> Result<()> {
        match command.syntax {
            IfSyntax::ThenFi { .. } => self.format_then_fi_if(command),
            IfSyntax::Brace { .. } => self.format_brace_if(command),
        }
    }

    fn format_then_fi_if(&mut self, command: &IfCommand) -> Result<()> {
        let source = self.source();
        let (then_span, syntax_fi_span) = match command.syntax {
            IfSyntax::ThenFi { then_span, fi_span } => (then_span, fi_span),
            IfSyntax::Brace { .. } => unreachable!("brace if cannot be formatted as then/fi"),
        };
        let fi_span = if_close_span(source, self.source_map(), command).unwrap_or(syntax_fi_span);
        let fi_upper_bound = fi_span.start.offset;

        if command.elif_branches.is_empty()
            && let Some(raw_condition) = raw_grouped_if_condition(command, then_span, source)
        {
            self.write_text("if");
            self.write_text(&raw_condition);
            self.write_text("then");
            let then_upper_bound = if_branch_upper_bound(command, 0, source, self.source_map());
            self.write_sequence_open_suffix(&command.then_branch, Some(then_upper_bound));
            let preserve_then_open_blank = body_has_blank_line_after_open(
                source,
                self.source_map(),
                then_span.end.offset,
                &command.then_branch,
            );
            self.format_body_with_upper_bound_and_open_blank(
                &command.then_branch,
                Some(then_upper_bound),
                preserve_then_open_blank,
            )?;
            self.write_unmodeled_branch_background_terminator(
                &command.then_branch,
                then_upper_bound,
            );
            if let Some(body) = &command.else_branch {
                if if_next_branch_has_blank_line_before_keyword(command, 0, source) {
                    self.newline();
                }
                self.newline();
                self.write_text("else");
                let body_upper_bound = fi_upper_bound;
                self.write_sequence_open_suffix(body, Some(body_upper_bound));
                let preserve_else_open_blank = body_has_blank_line_after_keyword(
                    source,
                    self.source_map(),
                    command.span.start.offset,
                    "else",
                    body,
                );
                self.format_body_with_upper_bound_and_open_blank(
                    body,
                    Some(body_upper_bound),
                    preserve_else_open_blank,
                )?;
                self.write_unmodeled_branch_background_terminator(body, body_upper_bound);
            }
            if self.if_final_branch_has_blank_line_before_fi(command, then_upper_bound) {
                self.newline();
            }
            self.newline();
            self.write_text("fi");
            self.write_close_suffix_after_span(Some(fi_span));
            return Ok(());
        }

        if if_condition_starts_after_keyword(command)
            || if_condition_has_explicit_statement_break(command, then_span, source)
        {
            self.write_text("if");
            self.newline();
            self.with_indent(|formatter| {
                formatter.format_stmt_sequence(&command.condition, Some(then_span.start.offset))
            })?;
            self.newline();
            self.write_text("then");
            let then_upper_bound = if_branch_upper_bound(command, 0, source, self.source_map());
            self.write_sequence_open_suffix(&command.then_branch, Some(then_upper_bound));
            let preserve_then_open_blank = body_has_blank_line_after_open(
                source,
                self.source_map(),
                then_span.end.offset,
                &command.then_branch,
            );
            self.format_body_with_upper_bound_and_open_blank(
                &command.then_branch,
                Some(then_upper_bound),
                preserve_then_open_blank,
            )?;
            self.write_unmodeled_branch_background_terminator(
                &command.then_branch,
                then_upper_bound,
            );
            for (index, (condition, body)) in command.elif_branches.iter().enumerate() {
                if if_next_branch_has_blank_line_before_keyword(command, index, source) {
                    self.newline();
                }
                let preserve_blank_after_prefix =
                    if_branch_prefix_comments_have_blank_line_before_keyword(
                        command, index, source,
                    );
                self.emit_branch_prefix_comments(command, index);
                self.newline();
                if preserve_blank_after_prefix {
                    self.newline();
                }
                let condition_prefix_comments =
                    self.elif_condition_prefix_comments(command, index, condition);
                if condition_keyword_on_previous_non_empty_line(condition, source, "elif")
                    || elif_condition_has_explicit_statement_break(condition, body, source)
                    || !condition_prefix_comments.is_empty()
                {
                    self.write_multiline_elif_header(condition, body)?;
                } else {
                    self.write_text("elif ");
                    self.format_inline_stmts(condition)?;
                    self.write_text(self.then_separator_for_condition(condition));
                }
                let body_upper_bound =
                    if_branch_upper_bound(command, index + 1, source, self.source_map());
                self.write_sequence_open_suffix(body, Some(body_upper_bound));
                let preserve_elif_open_blank = body_has_blank_line_after_keyword(
                    source,
                    self.source_map(),
                    condition.span.start.offset,
                    "then",
                    body,
                );
                self.format_body_with_upper_bound_and_open_blank(
                    body,
                    Some(body_upper_bound),
                    preserve_elif_open_blank,
                )?;
                self.write_unmodeled_branch_background_terminator(body, body_upper_bound);
            }
            if let Some(body) = &command.else_branch {
                if if_next_branch_has_blank_line_before_keyword(
                    command,
                    command.elif_branches.len(),
                    source,
                ) {
                    self.newline();
                }
                let preserve_blank_after_prefix =
                    if_branch_prefix_comments_have_blank_line_before_keyword(
                        command,
                        command.elif_branches.len(),
                        source,
                    );
                self.emit_branch_prefix_comments(command, command.elif_branches.len());
                self.newline();
                if preserve_blank_after_prefix {
                    self.newline();
                }
                self.write_text("else");
                let body_upper_bound = fi_upper_bound;
                self.write_sequence_open_suffix(body, Some(body_upper_bound));
                let preserve_else_open_blank = body_has_blank_line_after_keyword(
                    source,
                    self.source_map(),
                    command.span.start.offset,
                    "else",
                    body,
                );
                self.format_body_with_upper_bound_and_open_blank(
                    body,
                    Some(body_upper_bound),
                    preserve_else_open_blank,
                )?;
                self.write_unmodeled_branch_background_terminator(body, body_upper_bound);
            }
            if self.if_final_branch_has_blank_line_before_fi(command, then_upper_bound) {
                self.newline();
            }
            self.newline();
            self.write_text("fi");
            self.write_close_suffix_after_span(Some(fi_span));
            return Ok(());
        }

        self.write_text("if ");
        self.format_inline_stmts(&command.condition)?;
        let then_separator = self.then_separator_for_condition(&command.condition);
        if command.elif_branches.is_empty()
            && command.else_branch.is_none()
            && self.can_inline_body_with_upper_bound(
                &command.then_branch,
                command.span,
                Some(fi_upper_bound),
            )
        {
            self.write_text(then_separator);
            self.write_space();
            self.format_inline_stmts(&command.then_branch)?;
            self.write_text("; fi");
            self.write_close_suffix_after_span(Some(fi_span));
            return Ok(());
        }
        if command.elif_branches.is_empty()
            && let Some(else_branch) = &command.else_branch
            && self.can_inline_body_with_upper_bound(
                &command.then_branch,
                command.span,
                Some(fi_upper_bound),
            )
            && self.can_inline_body_with_upper_bound(
                else_branch,
                command.span,
                Some(fi_upper_bound),
            )
        {
            self.write_text(then_separator);
            self.write_space();
            self.format_inline_stmts(&command.then_branch)?;
            self.write_text("; else ");
            self.format_inline_stmts(else_branch)?;
            self.write_text("; fi");
            self.write_close_suffix_after_span(Some(fi_span));
            return Ok(());
        }
        if command.elif_branches.is_empty()
            && let Some(else_branch) = &command.else_branch
            && self.can_inline_body_with_upper_bound(
                &command.then_branch,
                command.span,
                Some(fi_upper_bound),
            )
            && !self.can_inline_body_with_upper_bound(
                else_branch,
                command.span,
                Some(fi_upper_bound),
            )
            && !self.options().compact_layout()
        {
            self.write_text(then_separator);
            self.write_space();
            self.format_inline_stmts(&command.then_branch)?;
            self.write_text("; else");
            let body_upper_bound = fi_upper_bound;
            self.write_sequence_open_suffix(else_branch, Some(body_upper_bound));
            let preserve_else_open_blank = body_has_blank_line_after_keyword(
                source,
                self.source_map(),
                command.span.start.offset,
                "else",
                else_branch,
            );
            self.format_body_with_upper_bound_and_open_blank(
                else_branch,
                Some(body_upper_bound),
                preserve_else_open_blank,
            )?;
            self.write_unmodeled_branch_background_terminator(else_branch, body_upper_bound);
            let then_upper_bound = if_branch_upper_bound(command, 0, source, self.source_map());
            if self.if_final_branch_has_blank_line_before_fi(command, then_upper_bound) {
                self.newline();
            }
            self.newline();
            self.write_text("fi");
            self.write_close_suffix_after_span(Some(fi_span));
            return Ok(());
        }
        if command.elif_branches.is_empty()
            && command.else_branch.is_none()
            && self.then_branch_starts_with_inline_if(command, then_span, fi_span)
        {
            self.write_text(then_separator);
            self.write_space();
            self.format_stmt(&command.then_branch[0])?;
            self.write_text("; fi");
            self.write_close_suffix_after_span(Some(fi_span));
            return Ok(());
        }
        if self.can_inline_if_chain(command, fi_span) {
            self.write_text(then_separator);
            self.write_space();
            self.format_inline_stmts(&command.then_branch)?;
            for (condition, body) in &command.elif_branches {
                self.write_text("; elif ");
                self.format_inline_stmts(condition)?;
                self.write_text(self.then_separator_for_condition(condition));
                self.write_space();
                self.format_inline_stmts(body)?;
            }
            if let Some(else_branch) = &command.else_branch {
                self.write_text("; else ");
                self.format_inline_stmts(else_branch)?;
            }
            self.write_text("; fi");
            self.write_close_suffix_after_span(Some(fi_span));
            return Ok(());
        }

        self.write_text(then_separator);
        let then_upper_bound = if_branch_upper_bound(command, 0, source, self.source_map());
        self.write_sequence_open_suffix(&command.then_branch, Some(then_upper_bound));
        let preserve_then_open_blank = body_has_blank_line_after_open(
            source,
            self.source_map(),
            then_span.end.offset,
            &command.then_branch,
        );
        self.format_body_with_upper_bound_and_open_blank(
            &command.then_branch,
            Some(then_upper_bound),
            preserve_then_open_blank,
        )?;
        self.write_unmodeled_branch_background_terminator(&command.then_branch, then_upper_bound);
        for (index, (condition, body)) in command.elif_branches.iter().enumerate() {
            if self.options().compact_layout() {
                self.write_text("; elif ");
                self.format_inline_stmts(condition)?;
                self.write_text(self.then_separator_for_condition(condition));
            } else {
                if if_next_branch_has_blank_line_before_keyword(command, index, source) {
                    self.newline();
                }
                let preserve_blank_after_prefix =
                    if_branch_prefix_comments_have_blank_line_before_keyword(
                        command, index, source,
                    );
                self.emit_branch_prefix_comments(command, index);
                self.newline();
                if preserve_blank_after_prefix {
                    self.newline();
                }
                let condition_prefix_comments =
                    self.elif_condition_prefix_comments(command, index, condition);
                if condition_keyword_on_previous_non_empty_line(condition, source, "elif")
                    || elif_condition_has_explicit_statement_break(condition, body, source)
                    || !condition_prefix_comments.is_empty()
                {
                    self.write_multiline_elif_header(condition, body)?;
                } else {
                    self.write_text("elif ");
                    self.format_inline_stmts(condition)?;
                    self.write_text(self.then_separator_for_condition(condition));
                }
            }
            let body_upper_bound =
                if_branch_upper_bound(command, index + 1, source, self.source_map());
            self.write_sequence_open_suffix(body, Some(body_upper_bound));
            let preserve_elif_open_blank = body_has_blank_line_after_keyword(
                source,
                self.source_map(),
                condition.span.start.offset,
                "then",
                body,
            );
            self.format_body_with_upper_bound_and_open_blank(
                body,
                Some(body_upper_bound),
                preserve_elif_open_blank,
            )?;
            self.write_unmodeled_branch_background_terminator(body, body_upper_bound);
        }
        if let Some(body) = &command.else_branch {
            if self.options().compact_layout() {
                self.write_text("; else");
            } else {
                if if_next_branch_has_blank_line_before_keyword(
                    command,
                    command.elif_branches.len(),
                    source,
                ) {
                    self.newline();
                }
                let preserve_blank_after_prefix =
                    if_branch_prefix_comments_have_blank_line_before_keyword(
                        command,
                        command.elif_branches.len(),
                        source,
                    );
                self.emit_branch_prefix_comments(command, command.elif_branches.len());
                self.newline();
                if preserve_blank_after_prefix {
                    self.newline();
                }
                if self.can_inline_else_branch_close(command, body, fi_span) {
                    self.write_text("else ");
                    self.format_inline_stmts(body)?;
                    self.write_text("; fi");
                    self.write_close_suffix_after_span(Some(fi_span));
                    return Ok(());
                }
                self.write_text("else");
            }
            let body_upper_bound = fi_upper_bound;
            self.write_sequence_open_suffix(body, Some(body_upper_bound));
            let preserve_else_open_blank = body_has_blank_line_after_keyword(
                source,
                self.source_map(),
                command.span.start.offset,
                "else",
                body,
            );
            self.format_body_with_upper_bound_and_open_blank(
                body,
                Some(body_upper_bound),
                preserve_else_open_blank,
            )?;
            self.write_unmodeled_branch_background_terminator(body, body_upper_bound);
        }
        if self.options().compact_layout() {
            self.write_text("; fi");
            self.write_close_suffix_after_span(Some(fi_span));
        } else {
            if self.if_final_branch_has_blank_line_before_fi(command, then_upper_bound) {
                self.newline();
            }
            self.newline();
            self.write_text("fi");
            self.write_close_suffix_after_span(Some(fi_span));
        }
        Ok(())
    }

    fn can_inline_else_branch_close(
        &self,
        command: &IfCommand,
        body: &StmtSeq,
        fi_span: Span,
    ) -> bool {
        let [stmt] = body.as_slice() else {
            return false;
        };
        if matches!(stmt.terminator, Some(StmtTerminator::Background(_)))
            || !self.can_inline_stmt(stmt)
            || self
                .facts()
                .sequence(body, Some(fi_span.start.offset))
                .has_comments()
        {
            return false;
        };
        let Some((_, else_offset)) =
            if_next_branch_region(command, command.elif_branches.len(), self.source())
        else {
            return false;
        };
        let else_line = self.source_map().line_number_for_offset(else_offset);
        let body_line = stmt_span(stmt).start.line;
        else_line == body_line && body_line == fi_span.start.line
    }

    fn can_inline_if_chain(&self, command: &IfCommand, fi_span: Span) -> bool {
        if command.elif_branches.is_empty() || command.span.start.line != fi_span.end.line {
            return false;
        }

        let source = self.source();
        if !self.can_inline_body_with_upper_bound(
            &command.then_branch,
            command.span,
            Some(if_branch_upper_bound(command, 0, source, self.source_map())),
        ) {
            return false;
        }

        for (index, (_, body)) in command.elif_branches.iter().enumerate() {
            if !self.can_inline_body_with_upper_bound(
                body,
                command.span,
                Some(if_branch_upper_bound(
                    command,
                    index + 1,
                    source,
                    self.source_map(),
                )),
            ) {
                return false;
            }
        }

        command.else_branch.as_ref().is_none_or(|body| {
            self.can_inline_body_with_upper_bound(body, command.span, Some(fi_span.start.offset))
        })
    }

    fn then_branch_starts_with_inline_if(
        &self,
        command: &IfCommand,
        then_span: Span,
        fi_span: Span,
    ) -> bool {
        if command.span.start.line != fi_span.end.line {
            return false;
        }
        let [stmt] = command.then_branch.as_slice() else {
            return false;
        };
        if stmt.negated || !stmt.redirects.is_empty() || stmt.terminator.is_some() {
            return false;
        }
        let Command::Compound(CompoundCommand::If(inner)) = &stmt.command else {
            return false;
        };
        matches!(inner.syntax, IfSyntax::ThenFi { .. })
            && then_span.end.line == inner.span.start.line
            && !self
                .facts()
                .sequence(
                    &command.then_branch,
                    Some(if_branch_upper_bound(
                        command,
                        0,
                        self.source(),
                        self.source_map(),
                    )),
                )
                .has_comments()
    }

    fn if_final_branch_has_blank_line_before_fi(
        &self,
        command: &IfCommand,
        then_upper_bound: usize,
    ) -> bool {
        let _ = then_upper_bound;
        let body = command
            .else_branch
            .as_ref()
            .or_else(|| command.elif_branches.last().map(|(_, body)| body))
            .unwrap_or(&command.then_branch);
        !body.is_empty()
            && source_has_blank_line_before_last_keyword_after(
                self.source(),
                sequence_close_gap_start(body, self.source()),
                command.span,
                "fi",
            )
    }

    fn emit_branch_prefix_comments(&mut self, command: &IfCommand, branch_index: usize) {
        let Some((start, end)) = if_next_branch_region(command, branch_index, self.source()) else {
            return;
        };
        let comments = branch_prefix_comments(self.source(), start, end)
            .into_iter()
            .map(|comment| {
                (
                    self.source_map().line_number_for_offset(comment.offset),
                    comment.text,
                    comment.source_indent,
                )
            })
            .collect::<Vec<_>>();
        if comments.is_empty() {
            return;
        }
        let disabled_branch_block = branch_prefix_comments_use_disabled_body_indent(&comments);
        self.newline();
        for (index, (line, text, _)) in comments.iter().enumerate() {
            if disabled_branch_block {
                self.with_indent(|formatter| formatter.write_text(text));
            } else {
                self.write_text(text);
            }
            if let Some((next_line, _, _)) = comments.get(index + 1) {
                self.write_line_breaks(line_gap_break_count(*line, *next_line));
            }
        }
    }

    fn elif_condition_prefix_comments(
        &self,
        command: &IfCommand,
        branch_index: usize,
        condition: &StmtSeq,
    ) -> Vec<BranchPrefixComment> {
        let Some((_, keyword_offset)) = if_next_branch_region(command, branch_index, self.source())
        else {
            return Vec::new();
        };
        let Some(first) = condition.first() else {
            return Vec::new();
        };
        let condition_start = stmt_span(first).start.offset;
        if keyword_offset >= condition_start {
            return Vec::new();
        }

        self.own_line_comments_in_region(keyword_offset, condition_start)
    }

    fn write_multiline_elif_header(&mut self, condition: &StmtSeq, body: &StmtSeq) -> Result<()> {
        self.write_text("elif");
        self.newline();
        self.with_indent(|formatter| {
            formatter.format_stmt_sequence(condition, Some(body.span.start.offset))
        })?;
        self.newline();
        self.write_text("then");
        Ok(())
    }

    fn write_sequence_open_suffix(&mut self, commands: &StmtSeq, upper_bound: Option<usize>) {
        let Some(span) = self
            .facts()
            .sequence(commands, upper_bound)
            .group_open_suffix_span()
        else {
            return;
        };
        self.write_suffix_comment_after_span(span, false);
    }

    fn write_unmodeled_branch_background_terminator(&mut self, body: &StmtSeq, upper_bound: usize) {
        let Some(operator) = unmodeled_branch_background_operator(body, upper_bound, self.source())
        else {
            return;
        };
        self.write_space();
        self.write_text(operator);
    }

    fn format_brace_if(&mut self, command: &IfCommand) -> Result<()> {
        let source = self.source();
        self.write_text("if ");
        self.format_inline_stmts(&command.condition)?;
        self.write_space();
        self.format_brace_group(
            &command.then_branch,
            Some(if_branch_upper_bound(command, 0, source, self.source_map())),
        )?;
        for (index, (condition, body)) in command.elif_branches.iter().enumerate() {
            self.write_text(" elif ");
            self.format_inline_stmts(condition)?;
            self.write_space();
            self.format_brace_group(
                body,
                Some(if_branch_upper_bound(
                    command,
                    index + 1,
                    source,
                    self.source_map(),
                )),
            )?;
        }
        if let Some(body) = &command.else_branch {
            self.write_text(" else ");
            self.format_brace_group(body, Some(command.span.end.offset))?;
        }
        Ok(())
    }

    fn format_for(&mut self, command: &ForCommand) -> Result<()> {
        self.write_text("for ");
        for (index, target) in command.targets.iter().enumerate() {
            if index > 0 {
                self.write_space();
            }
            self.write_word(&target.word);
        }

        match command.syntax {
            ForSyntax::InDoDone { in_span, .. } => {
                if let Some(words) = &command.words {
                    self.write_text(" in");
                    self.write_word_list_preserving_breaks_after(
                        words,
                        in_span.map(|span| span.end.offset),
                    );
                }
                let close_span = match command.syntax {
                    ForSyntax::InDoDone { done_span, .. } => done_close_span(
                        self.source(),
                        self.source_map(),
                        command.span,
                        Some(done_span),
                    ),
                    _ => None,
                };
                self.format_do_done_body(&command.body, command.span, close_span, "done")?;
            }
            ForSyntax::InDirect { in_span } => {
                if let Some(words) = &command.words {
                    self.write_text(" in");
                    self.write_word_list_preserving_breaks_after(
                        words,
                        in_span.map(|span| span.end.offset),
                    );
                }
                self.write_space();
                self.format_inline_stmts(&command.body)?;
            }
            ForSyntax::ParenDoDone { .. } => {
                self.write_text(" (");
                for (index, word) in command
                    .words
                    .iter()
                    .flat_map(|words| words.iter())
                    .enumerate()
                {
                    if index > 0 {
                        self.write_space();
                    }
                    self.write_word(word);
                }
                self.write_text(")");
                let close_span = match command.syntax {
                    ForSyntax::ParenDoDone { done_span, .. } => done_close_span(
                        self.source(),
                        self.source_map(),
                        command.span,
                        Some(done_span),
                    ),
                    _ => None,
                };
                self.format_do_done_body(&command.body, command.span, close_span, "done")?;
            }
            ForSyntax::ParenDirect { .. } => {
                self.write_text(" (");
                for (index, word) in command
                    .words
                    .iter()
                    .flat_map(|words| words.iter())
                    .enumerate()
                {
                    if index > 0 {
                        self.write_space();
                    }
                    self.write_word(word);
                }
                self.write_text(") ");
                self.format_inline_stmts(&command.body)?;
            }
            ForSyntax::InBrace { in_span, .. } => {
                if let Some(words) = &command.words {
                    self.write_text(" in");
                    self.write_word_list_preserving_breaks_after(
                        words,
                        in_span.map(|span| span.end.offset),
                    );
                }
                self.write_text("; ");
                self.format_brace_group(&command.body, Some(command.span.end.offset))?;
            }
            ForSyntax::ParenBrace { .. } => {
                self.write_text(" (");
                for (index, word) in command
                    .words
                    .iter()
                    .flat_map(|words| words.iter())
                    .enumerate()
                {
                    if index > 0 {
                        self.write_space();
                    }
                    self.write_word(word);
                }
                self.write_text("); ");
                self.format_brace_group(&command.body, Some(command.span.end.offset))?;
            }
        }
        Ok(())
    }

    fn format_repeat(&mut self, command: &RepeatCommand) -> Result<()> {
        self.write_text("repeat ");
        self.write_word(&command.count);
        match command.syntax {
            RepeatSyntax::DoDone { .. } => {
                let close_span = match command.syntax {
                    RepeatSyntax::DoDone { done_span, .. } => done_close_span(
                        self.source(),
                        self.source_map(),
                        command.span,
                        Some(done_span),
                    ),
                    _ => None,
                };
                self.format_do_done_body(&command.body, command.span, close_span, "done")?;
            }
            RepeatSyntax::Direct => {
                self.write_space();
                self.format_inline_stmts(&command.body)?;
            }
            RepeatSyntax::Brace { .. } => {
                self.write_space();
                self.format_brace_group(&command.body, Some(command.span.end.offset))?;
            }
        }
        Ok(())
    }

    fn format_foreach(&mut self, command: &ForeachCommand) -> Result<()> {
        self.write_text("foreach ");
        self.write_text(command.variable.as_ref());
        match command.syntax {
            ForeachSyntax::ParenBrace { .. } => {
                self.write_text(" (");
                for (index, word) in command.words.iter().enumerate() {
                    if index > 0 {
                        self.write_space();
                    }
                    self.write_word(word);
                }
                self.write_text(") ");
                self.format_brace_group(&command.body, Some(command.span.end.offset))?;
            }
            ForeachSyntax::InDoDone { .. } => {
                self.write_text(" in");
                self.write_word_list_preserving_breaks(&command.words);
                let close_span = match command.syntax {
                    ForeachSyntax::InDoDone { done_span, .. } => done_close_span(
                        self.source(),
                        self.source_map(),
                        command.span,
                        Some(done_span),
                    ),
                    _ => None,
                };
                self.format_do_done_body(&command.body, command.span, close_span, "done")?;
            }
        }
        Ok(())
    }

    fn format_select(&mut self, command: &SelectCommand) -> Result<()> {
        self.write_text("select ");
        self.write_text(command.variable.as_ref());
        self.write_text(" in");
        self.write_word_list_preserving_breaks(&command.words);
        let close_span = done_close_span(self.source(), self.source_map(), command.span, None);
        self.format_do_done_body(&command.body, command.span, close_span, "done")?;
        Ok(())
    }

    fn format_while(&mut self, command: &WhileCommand) -> Result<()> {
        let close_span = done_close_span(self.source(), self.source_map(), command.span, None);
        if loop_condition_starts_after_keyword(&command.condition, command.span)
            || loop_condition_has_multiple_commands(&command.condition)
        {
            self.write_text("while");
            self.newline();
            let condition_upper_bound =
                branch_open_keyword_start(&command.body, self.source(), "do");
            self.with_indent(|formatter| {
                formatter.format_stmt_sequence(&command.condition, condition_upper_bound)
            })?;
            self.newline();
            return self.format_split_do_done_body(&command.body, command.span, close_span, "done");
        }

        self.write_text("while ");
        self.format_inline_stmts(&command.condition)?;
        self.format_do_done_body(&command.body, command.span, close_span, "done")
    }

    fn format_until(&mut self, command: &UntilCommand) -> Result<()> {
        let close_span = done_close_span(self.source(), self.source_map(), command.span, None);
        if loop_condition_starts_after_keyword(&command.condition, command.span)
            || loop_condition_has_multiple_commands(&command.condition)
        {
            self.write_text("until");
            self.newline();
            let condition_upper_bound =
                branch_open_keyword_start(&command.body, self.source(), "do");
            self.with_indent(|formatter| {
                formatter.format_stmt_sequence(&command.condition, condition_upper_bound)
            })?;
            self.newline();
            return self.format_split_do_done_body(&command.body, command.span, close_span, "done");
        }

        self.write_text("until ");
        self.format_inline_stmts(&command.condition)?;
        self.format_do_done_body(&command.body, command.span, close_span, "done")
    }

    fn format_case(&mut self, command: &CaseCommand) -> Result<()> {
        if !self.options().compact_layout()
            && case_command_was_inline_in_source(command, self.source())
            && self.can_format_case_inline(command)
        {
            return self.format_inline_case(command);
        }

        let esac_span =
            last_shell_keyword_span(self.source(), self.source_map(), command.span, "esac");
        let case_body_fallback = esac_span
            .map(|span| span.start.offset)
            .unwrap_or(command.span.end.offset);
        self.write_text("case ");
        self.write_word(&command.word);
        self.write_text(" in");
        self.write_case_open_suffix(command);
        if self.options().compact_layout() {
            for item in &command.cases {
                self.write_space();
                self.format_case_item(item, case_item_body_upper_bound(item, case_body_fallback))?;
            }
            self.write_text(" esac");
            self.write_close_suffix_after_span(esac_span);
        } else {
            let header_item_count =
                self.format_case_items_on_header_line(command, case_body_fallback)?;
            for (offset, item) in command.cases[header_item_count..].iter().enumerate() {
                let index = header_item_count + offset;
                self.newline();
                if header_item_count == 0
                    && index == 0
                    && case_has_blank_line_after_in(command, self.source())
                {
                    self.newline();
                }
                if index > 0
                    && case_item_has_blank_line_before(
                        &command.cases[index - 1],
                        item,
                        self.source(),
                    )
                {
                    self.newline();
                }
                self.format_case_item(item, case_item_body_upper_bound(item, case_body_fallback))?;
            }
            let suffix_comments = self.case_suffix_comments_before_esac(command, esac_span);
            if suffix_comments.is_empty() {
                if case_has_blank_line_before_esac(command, self.source()) {
                    self.newline();
                }
                self.newline();
            } else {
                self.emit_case_suffix_comments_before_esac(command, &suffix_comments, esac_span);
            }
            self.write_text("esac");
            self.write_close_suffix_after_span(esac_span);
        }
        Ok(())
    }

    fn format_case_items_on_header_line(
        &mut self,
        command: &CaseCommand,
        case_body_fallback: usize,
    ) -> Result<usize> {
        let mut item_count = 0;
        for item in &command.cases {
            if !case_item_pattern_starts_on_case_header(command, item) {
                break;
            }
            let upper_bound = case_item_body_upper_bound(item, case_body_fallback);
            if !self.case_item_prefix_comments(item, upper_bound).is_empty() {
                break;
            }
            self.write_space();
            self.format_case_item(item, upper_bound)?;
            item_count += 1;
        }
        Ok(item_count)
    }

    fn write_case_open_suffix(&mut self, command: &CaseCommand) {
        let Some(first_item) = command.cases.first() else {
            return;
        };
        let Some(first_pattern) = first_item.patterns.first() else {
            return;
        };
        let source = self.source();
        let start = command.word.span.end.offset.min(source.len());
        let end = first_pattern.span.start.offset.min(source.len());
        let Some(between) = source.get(start..end) else {
            return;
        };
        let line_end = between.find('\n').unwrap_or(between.len());
        let header = &between[..line_end];
        let header = header.trim_start_matches([' ', '\t']);
        let Some(suffix) = header.strip_prefix("in") else {
            return;
        };
        if suffix.trim_start().starts_with('#') {
            self.write_space();
            self.write_text(suffix.trim_start().trim_end_matches([' ', '\t', '\r']));
        }
    }

    fn format_case_item(&mut self, item: &CaseItem, upper_bound: Option<usize>) -> Result<()> {
        let base_indent =
            usize::from(!self.options().compact_layout() && self.options().switch_case_indent());
        let first_pattern_start = item
            .patterns
            .first()
            .map(|pattern| pattern.span.start.offset);
        let prefix_comments = self.case_item_prefix_comments(item, upper_bound);
        if let Some(first_pattern) = item.patterns.first()
            && !prefix_comments.is_empty()
        {
            self.emit_case_item_prefix_comments(&prefix_comments, first_pattern, base_indent);
        }

        if base_indent > 0 {
            self.write_case_prefix(base_indent);
        }
        for (index, word) in item.patterns.iter().enumerate() {
            if index > 0 {
                let previous = &item.patterns[index - 1];
                if has_newline_between_offsets(
                    self.source(),
                    previous.span.end.offset,
                    word.span.start.offset,
                ) {
                    self.write_text(" |");
                    self.line_continuation();
                    self.write_indent_units(1);
                } else {
                    self.write_text(" | ");
                }
            }
            self.write_case_pattern(item, word);
        }
        self.write_text(")");
        let pattern_suffix_comment = self.case_item_pattern_suffix_comment(item, upper_bound);
        if let Some(comment) = &pattern_suffix_comment {
            let current_code_column = self.column.saturating_sub(self.line_indent_column);
            let mut padding = trailing_comment_padding(self.source(), comment, current_code_column);
            if trailing_comment_alignment_column(self.source(), comment).is_some() {
                padding += 1;
            }
            for _ in 0..padding {
                self.write_space();
            }
            self.write_comment(comment);
        }

        if item.body.is_empty() {
            let body_has_comments = self
                .facts()
                .sequence(&item.body, upper_bound)
                .has_comments();
            let comments = self.empty_case_item_body_comments(item);
            if (body_has_comments || !comments.is_empty()) && !self.options().compact_layout() {
                self.newline();
                if comments.is_empty() {
                    self.with_extra_prefix_indent(base_indent + 1, |formatter| {
                        formatter.format_stmt_sequence(&item.body, upper_bound)
                    })?;
                } else {
                    self.with_extra_prefix_indent(base_indent + 1, |formatter| {
                        for (index, comment) in comments.iter().enumerate() {
                            if index > 0 {
                                formatter.newline();
                            }
                            formatter.write_text(comment);
                        }
                    });
                }
                self.newline();
                self.write_case_prefix(base_indent + 1);
                self.write_case_terminator(item);
                return Ok(());
            }
            self.write_space();
            self.write_case_terminator(item);
        } else if self.options().compact_layout() {
            self.write_space();
            self.format_stmt_sequence_with_leading_filter(
                &item.body,
                upper_bound,
                first_pattern_start,
            )?;
            self.write_text("; ");
            self.write_case_terminator(item);
        } else {
            let body_sequence = self.facts().sequence(&item.body, upper_bound);
            let pattern_line = item.patterns.last().map(|pattern| pattern.span.end.line);
            let body_has_later_comments = pattern_line.is_some_and(|pattern_line| {
                (0..item.body.len()).any(|index| {
                    body_sequence
                        .leading_for(index)
                        .iter()
                        .chain(body_sequence.trailing_for(index))
                        .any(|comment| comment.line() > pattern_line)
                }) || body_sequence
                    .dangling()
                    .iter()
                    .any(|comment| comment.line() > pattern_line)
            });
            let first_body_line = body_sequence.first_rendered_line_for(0);
            let item_was_inline_in_source = self.facts().case_item_was_inline_in_source(item)
                || case_item_pattern_body_terminator_was_inline_in_source(item, self.source());
            if base_indent == 0
                && item.body.len() == 1
                && case_item_single_body_stmt_can_inline(&item.body[0])
                && (item_was_inline_in_source
                    || (pattern_suffix_comment.is_some()
                        && !body_has_later_comments
                        && case_item_body_can_share_terminator(item)
                        && case_item_body_terminator_was_inline_in_source(item))
                    || (!body_has_later_comments
                        && case_item_body_was_inline_without_terminator(item))
                    || (!body_has_later_comments
                        && case_item_started_inline_without_terminator(item)))
            {
                if pattern_suffix_comment.is_some() && !item_was_inline_in_source {
                    self.newline();
                    self.with_extra_prefix_indent(base_indent + 1, |formatter| {
                        formatter.format_stmt(&item.body[0])
                    })?;
                } else {
                    self.write_space();
                    self.format_stmt(&item.body[0])?;
                }
                self.write_space();
                self.write_case_terminator(item);
                return Ok(());
            }

            self.newline();
            let first_body_stmt_line = item
                .body
                .first()
                .map(|stmt| {
                    stmt_render_start_line(stmt, self.source(), self.source_map(), self.options())
                })
                .unwrap_or(first_body_line);
            if (case_item_pattern_close_paren_on_own_line(item, self.source())
                && !case_item_close_paren_shares_line_with_body(item, self.source()))
                || case_item_has_blank_line_after_pattern(
                    item,
                    self.source(),
                    first_body_line,
                    first_body_stmt_line,
                )
            {
                self.newline();
            }
            self.with_extra_prefix_indent(base_indent + 1, |formatter| {
                formatter.format_stmt_sequence_with_leading_filter(
                    &item.body,
                    upper_bound,
                    first_pattern_start,
                )
            })?;
            if case_item_has_blank_line_before_terminator(item, self.source()) {
                self.newline();
            }
            self.newline();
            self.write_case_prefix(base_indent + 1);
            self.write_case_terminator(item);
        }
        Ok(())
    }

    fn write_case_terminator(&mut self, item: &CaseItem) {
        self.write_text(case_terminator(item.terminator));
        if let Some(comment) = self.case_item_terminator_suffix_comment(item) {
            let current_code_column = self.column.saturating_sub(self.line_indent_column);
            let padding = trailing_comment_padding(self.source(), &comment, current_code_column);
            for _ in 0..padding {
                self.write_space();
            }
            self.write_comment(&comment);
        }
    }

    fn empty_case_item_body_comments(&self, item: &CaseItem) -> Vec<String> {
        if !item.body.is_empty() {
            return Vec::new();
        }
        let Some(end) = item.terminator_span.map(|span| span.start.offset) else {
            return Vec::new();
        };
        let start = item
            .patterns
            .last()
            .map(|pattern| pattern.span.end.offset)
            .unwrap_or(item.body.span.start.offset);
        let Some(slice) = self.source().get(start..end) else {
            return Vec::new();
        };

        slice
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim_start_matches([' ', '\t']);
                trimmed
                    .starts_with('#')
                    .then(|| trimmed.trim_end_matches([' ', '\t', '\r']).to_string())
            })
            .collect()
    }

    fn case_item_pattern_suffix_comment(
        &self,
        item: &CaseItem,
        upper_bound: Option<usize>,
    ) -> Option<SourceComment<'source>> {
        let source = self.source();
        let start = item.patterns.last()?.span.end.offset.min(source.len());
        let end = item
            .body
            .first()
            .map(|stmt| stmt_span(stmt).start.offset)
            .or_else(|| item.terminator_span.map(|span| span.start.offset))
            .or(upper_bound)
            .unwrap_or(source.len())
            .min(source.len());
        if start >= end {
            return None;
        }
        let between = source.get(start..end)?;
        let line = between.split_once('\n').map_or(between, |(line, _)| line);
        let comment_start = line.find('#')?;
        let before = &line[..comment_start];
        if !before.contains(')') {
            return None;
        }
        let comment = line[comment_start..].trim_end_matches([' ', '\t', '\r']);
        let absolute_start = start + comment_start;
        let absolute_end = absolute_start + comment.len();
        self.source_map()
            .source_comment_for_offsets(absolute_start, absolute_end)
    }

    fn case_item_terminator_suffix_comment(
        &self,
        item: &CaseItem,
    ) -> Option<SourceComment<'source>> {
        let span = item.terminator_span?;
        if span.start.line != span.end.line {
            return None;
        }
        let source = self.source();
        let start = span.end.offset.min(source.len());
        let suffix_source = source.get(start..)?;
        let line_end = suffix_source
            .find('\n')
            .map_or(source.len(), |offset| start + offset);
        let suffix = source.get(start..line_end)?;
        let leading_padding = suffix.len() - suffix.trim_start_matches([' ', '\t']).len();
        let comment = suffix[leading_padding..].trim_end_matches([' ', '\t', '\r']);
        if !comment.starts_with('#') {
            return None;
        }
        let absolute_start = start + leading_padding;
        let absolute_end = absolute_start + comment.len();
        self.source_map()
            .source_comment_for_offsets(absolute_start, absolute_end)
    }

    fn case_item_prefix_comments(
        &self,
        item: &CaseItem,
        upper_bound: Option<usize>,
    ) -> Vec<SourceComment<'source>> {
        let Some(first_pattern_start) = item
            .patterns
            .first()
            .map(|pattern| pattern.span.start.offset)
        else {
            return Vec::new();
        };
        self.facts()
            .sequence(&item.body, upper_bound)
            .leading_for(0)
            .iter()
            .copied()
            .filter(|comment| comment.span().start.offset < first_pattern_start)
            .collect()
    }

    fn case_suffix_comments_before_esac(
        &self,
        command: &CaseCommand,
        esac_span: Option<Span>,
    ) -> Vec<BranchPrefixComment> {
        let Some(last_item) = command.cases.last() else {
            return Vec::new();
        };
        let Some(start) = case_suffix_comment_region_start(last_item, self.source()) else {
            return Vec::new();
        };
        let end = esac_span
            .map(|span| span.start.offset)
            .unwrap_or(command.span.end.offset);
        self.own_line_comments_in_region(start, end)
    }

    fn emit_case_suffix_comments_before_esac(
        &mut self,
        command: &CaseCommand,
        comments: &[BranchPrefixComment],
        esac_span: Option<Span>,
    ) {
        let Some(mut previous_line) = command
            .cases
            .last()
            .and_then(case_suffix_comment_start_line)
        else {
            return;
        };
        let comment_indent = usize::from(self.options().switch_case_indent()) + 1;
        for comment in comments {
            let comment_line = self.source_map().line_number_for_offset(comment.offset);
            self.write_line_breaks(line_gap_break_count(previous_line, comment_line));
            self.with_extra_prefix_indent(comment_indent, |formatter| {
                formatter.write_text(&comment.text);
            });
            previous_line = comment_line;
        }
        let esac_line = esac_span
            .map(|span| span.start.line)
            .unwrap_or(command.span.end.line);
        self.write_line_breaks(line_gap_break_count(previous_line, esac_line));
    }

    fn emit_case_item_prefix_comments(
        &mut self,
        comments: &[SourceComment<'_>],
        first_pattern: &Pattern,
        base_indent: usize,
    ) {
        for (index, comment) in comments.iter().enumerate() {
            let disabled_case_pattern_context = comments[..index]
                .iter()
                .all(comment_looks_like_disabled_case_pattern);
            let extra_indent = base_indent
                + usize::from(case_prefix_comment_uses_body_indent(
                    self.source(),
                    comment,
                    first_pattern.span.start.offset,
                    disabled_case_pattern_context,
                ));
            self.with_extra_prefix_indent(extra_indent, |formatter| {
                formatter.write_comment(comment);
            });
            let target_line = comments
                .get(index + 1)
                .map(SourceComment::line)
                .unwrap_or(first_pattern.span.start.line);
            self.write_line_breaks(line_gap_break_count(comment.line(), target_line));
        }
    }

    fn with_extra_prefix_indent<T>(&mut self, levels: usize, f: impl FnOnce(&mut Self) -> T) -> T {
        self.indent_level += levels;
        let result = f(self);
        self.indent_level = self.indent_level.saturating_sub(levels);
        result
    }

    fn with_pipeline_continuation_indent<T>(
        &mut self,
        levels: usize,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let previous = self.pipeline_continuation_indent;
        self.pipeline_continuation_indent = levels;
        let result = f(self);
        self.pipeline_continuation_indent = previous;
        result
    }

    fn with_group_body_leading_filter<T>(
        &mut self,
        enabled: bool,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        if !enabled {
            return f(self);
        }
        let previous = self.filter_next_group_body_leading_before_open;
        self.filter_next_group_body_leading_before_open = true;
        let result = f(self);
        self.filter_next_group_body_leading_before_open = previous;
        result
    }

    fn format_brace_group(&mut self, commands: &StmtSeq, upper_bound: Option<usize>) -> Result<()> {
        let sequence_facts = self.facts().sequence(commands, upper_bound);
        let should_inline = sequence_facts.group_open_suffix_span().is_none()
            && self.group_has_inline_source_shape(commands, '{')
            && self.can_inline_group(commands, '{');
        if should_inline {
            self.write_text("{ ");
            self.format_inline_stmts(commands)?;
            self.write_text("; }");
            return Ok(());
        }
        self.format_group_with_upper_bound("{", "}", '{', commands, false, upper_bound)
    }

    fn format_subshell(&mut self, commands: &StmtSeq, upper_bound: Option<usize>) -> Result<()> {
        let sequence_facts = self.facts().sequence(commands, upper_bound);
        let should_inline = sequence_facts.group_open_suffix_span().is_none()
            && ((self.group_has_inline_source_shape(commands, '(')
                && self.can_inline_group(commands, '('))
                || self.can_inline_source_line_subshell(commands, upper_bound));
        if should_inline {
            self.write_text("(");
            self.format_inline_stmts(commands)?;
            self.write_text(")");
            return Ok(());
        }
        if self.can_format_multiline_subshell_inline(commands, upper_bound) {
            self.write_text("(");
            self.format_stmt_sequence(commands, upper_bound)?;
            self.write_text(")");
            return Ok(());
        }
        self.format_group_with_upper_bound("(", ")", '(', commands, false, upper_bound)
    }

    fn format_arithmetic(&mut self, command: &ArithmeticCommand) -> Result<()> {
        let expression_source = command
            .expr_span
            .and_then(|span| self.source().get(span.start.offset..span.end.offset));
        if let Some(expr) = command.expr_ast.as_ref()
            && !expression_source.is_some_and(|source| source.contains('\n'))
        {
            let mut body = self.take_scratch_buffer();
            render_arithmetic_expr_to_buf(&mut body, expr, self.source(), self.options());
            self.write_text("((");
            self.write_text(&body);
            self.write_text("))");
            self.restore_scratch_buffer(body);
            return Ok(());
        }
        let rendered = self
            .source()
            .get(command.span.start.offset..command.span.end.offset)
            .unwrap_or_default();
        self.write_text(&format_arithmetic_command_source(rendered));
        Ok(())
    }

    fn format_arithmetic_for(&mut self, command: &ArithmeticForCommand) -> Result<()> {
        let source = self.source();
        let init = slice_span(source, command.init_span);
        let condition = command
            .condition_span
            .map(|span| span.slice(source))
            .unwrap_or("");
        let step = command
            .step_span
            .map(|span| span.slice(source))
            .unwrap_or("");
        let init = format_arithmetic_for_clause_source(
            init,
            command.init_ast.as_ref(),
            source,
            self.options(),
        );
        let condition = format_arithmetic_for_clause_source(
            condition,
            command.condition_ast.as_ref(),
            source,
            self.options(),
        );
        let step = format_arithmetic_for_clause_source(
            step,
            command.step_ast.as_ref(),
            source,
            self.options(),
        );
        self.write_text("for ((");
        self.write_text(&init);
        self.write_text("; ");
        self.write_text(&condition);
        self.write_text("; ");
        self.write_text(&step);
        self.write_text("))");
        let close_span = done_close_span(self.source(), self.source_map(), command.span, None);
        self.format_do_done_body(&command.body, command.span, close_span, "done")
    }

    fn format_time(&mut self, command: &TimeCommand) -> Result<()> {
        if command.posix_format {
            self.write_text("time -p");
        } else {
            self.write_text("time");
        }
        if let Some(command) = &command.command {
            self.write_space();
            self.format_stmt(command)?;
            self.write_time_inner_trailing_comment(command);
        }
        Ok(())
    }

    fn write_time_inner_trailing_comment(&mut self, stmt: &Stmt) {
        if !time_inner_stmt_needs_trailing_comment(stmt) {
            return;
        }
        let Some(comment) = self.close_suffix_comment_after_span(stmt_format_span(stmt)) else {
            return;
        };
        self.emit_trailing_comments_for_stmt(&[comment]);
    }

    fn format_conditional(&mut self, command: &ConditionalCommand) -> Result<()> {
        self.write_text("[[ ");
        self.format_conditional_expr(&command.expression)?;
        let tight_close = self.conditional_needs_tight_close(&command.expression);
        self.write_text(if tight_close { "]]" } else { " ]]" });
        Ok(())
    }

    fn format_coproc(&mut self, command: &CoprocCommand) -> Result<()> {
        self.write_text("coproc");
        if command.name.as_str() != "COPROC" || command.name_span.is_some() {
            self.write_space();
            self.write_text(command.name.as_str());
        }
        self.write_space();
        self.format_stmt(&command.body)
    }

    fn format_always(&mut self, command: &AlwaysCommand) -> Result<()> {
        self.format_brace_group(&command.body, Some(command.span.end.offset))?;
        self.write_text(" always ");
        self.format_brace_group(&command.always_body, Some(command.span.end.offset))
    }

    fn format_function(&mut self, function: &FunctionDef) -> Result<()> {
        let header_comment = self.function_header_trailing_comment(function);
        self.format_named_function_header(function);
        if self.options().function_next_line() {
            self.newline();
            self.format_function_body(function.body.as_ref(), function.span.end.offset)
        } else {
            self.write_space();
            self.format_function_body_with_header_comment(
                function.body.as_ref(),
                function.span.end.offset,
                header_comment,
            )
        }
    }

    fn format_anonymous_function(&mut self, function: &AnonymousFunctionCommand) -> Result<()> {
        self.write_text(match function.surface {
            shuck_ast::AnonymousFunctionSurface::FunctionKeyword { .. } => "function",
            shuck_ast::AnonymousFunctionSurface::Parens { .. } => "()",
        });
        if self.options().function_next_line() {
            self.newline();
        } else {
            self.write_space();
        }
        self.format_function_body(function.body.as_ref(), function.span.end.offset)?;
        if !function.args.is_empty() {
            for argument in &function.args {
                self.write_space();
                self.write_word(argument);
            }
        }
        Ok(())
    }

    fn format_named_function_header(&mut self, function: &FunctionDef) {
        if function.header.entries.len() == 1
            && let Some(name) = function.header.entries[0].static_name.as_ref()
        {
            let source_map = self.source_map().clone();
            let mut rendered_entry = self.take_scratch_buffer();
            {
                let facts = self.facts();
                render_word_syntax_with_facts_to_buf(
                    &function.header.entries[0].word,
                    self.source(),
                    self.options(),
                    &source_map,
                    facts,
                    &mut rendered_entry,
                );
            }
            let classic_single_name = name.as_str() == rendered_entry;
            self.restore_scratch_buffer(rendered_entry);

            if classic_single_name {
                if function.uses_function_keyword() {
                    self.write_text("function ");
                }
                self.write_text(name.as_str());
                if function.has_trailing_parens() {
                    self.write_text("()");
                }
                return;
            }
        }

        if function.uses_function_keyword() {
            self.write_text("function");
            if !function.header.entries.is_empty() {
                self.write_space();
            }
        }
        for (index, entry) in function.header.entries.iter().enumerate() {
            if index > 0 {
                self.write_space();
            }
            self.write_word(&entry.word);
        }
        if function.has_trailing_parens() {
            self.write_text("()");
        }
    }

    fn format_function_body(&mut self, body: &Stmt, upper_bound: usize) -> Result<()> {
        self.format_function_body_with_header_comment(body, upper_bound, None)
    }

    fn format_function_body_with_header_comment(
        &mut self,
        body: &Stmt,
        upper_bound: usize,
        header_comment: Option<(Span, String)>,
    ) -> Result<()> {
        match body {
            Stmt {
                command: Command::Compound(CompoundCommand::BraceGroup(commands)),
                negated: false,
                redirects,
                terminator: None,
                ..
            } if redirects.is_empty() => {
                if let Some((_, comment)) = header_comment {
                    return self.format_function_brace_group_with_header_comment(
                        commands,
                        upper_bound,
                        &comment,
                    );
                }

                let should_inline = !self.options().function_next_line()
                    && self.group_has_inline_source_shape(commands, '{')
                    && self.can_inline_group(commands, '{');
                if should_inline {
                    self.write_text("{ ");
                    self.format_inline_stmts(commands)?;
                    self.write_text("; }");
                    Ok(())
                } else {
                    self.format_brace_group(commands, Some(upper_bound))
                }
            }
            Stmt {
                command: Command::Compound(CompoundCommand::Subshell(commands)),
                negated: false,
                redirects,
                terminator: None,
                ..
            } if redirects.is_empty() => {
                let should_inline = !self.options().function_next_line()
                    && self.group_has_inline_source_shape(commands, '(')
                    && self.can_inline_group(commands, '(');
                if should_inline {
                    self.write_text("(");
                    self.format_inline_stmts(commands)?;
                    self.write_text(")");
                    Ok(())
                } else {
                    self.format_subshell(commands, Some(upper_bound))
                }
            }
            _ => self.format_stmt(body),
        }
    }

    fn format_function_brace_group_with_header_comment(
        &mut self,
        commands: &StmtSeq,
        upper_bound: usize,
        header_comment: &str,
    ) -> Result<()> {
        self.write_text("{");
        let padding =
            self.function_header_comment_padding(commands, Some(upper_bound), self.column);
        for _ in 0..padding {
            self.write_space();
        }
        self.write_text(header_comment.trim_start());

        let open_suffix = self
            .facts()
            .sequence(commands, Some(upper_bound))
            .group_open_suffix_span()
            .map(|span| (span, span.slice(self.source()).trim_start().to_string()));

        if self.options().compact_layout() {
            if let Some((_, suffix)) = open_suffix {
                self.write_space();
                self.write_text(&suffix);
                self.write_text("; ");
            } else {
                self.write_space();
            }
            self.format_stmt_sequence(commands, Some(upper_bound))?;
        } else if commands.is_empty() {
            if let Some((_, suffix)) = open_suffix {
                self.newline();
                self.with_indent(|formatter| formatter.write_text(&suffix));
            }
        } else {
            self.newline();
            self.with_indent(|formatter| {
                if let Some((_, suffix)) = open_suffix {
                    formatter.write_text(&suffix);
                    formatter.newline();
                }
                formatter.format_stmt_sequence(commands, Some(upper_bound))
            })?;
        }

        self.finish_block("}");
        Ok(())
    }

    fn function_header_comment_padding(
        &self,
        commands: &StmtSeq,
        upper_bound: Option<usize>,
        header_column: usize,
    ) -> usize {
        if self
            .facts()
            .sequence(commands, upper_bound)
            .group_open_suffix_span()
            .is_some()
        {
            return 1;
        }
        if self
            .facts()
            .sequence(commands, upper_bound)
            .leading_for(0)
            .iter()
            .any(|comment| !comment.inline())
        {
            return 1;
        }
        let Some(target_code_column) = self.first_body_inline_comment_target_column(commands)
        else {
            return 1;
        };
        let body_indent_column = self.indent_column_for_level(self.indent_level + 1);
        body_indent_column
            .saturating_add(target_code_column)
            .saturating_sub(header_column)
            .max(1)
    }

    fn first_body_inline_comment_target_column(&self, commands: &StmtSeq) -> Option<usize> {
        let first = commands.first()?;
        let source = self.source();
        let start = stmt_span(first).start.offset.min(source.len());
        let (line_start, line_end) = line_bounds_for_offset(source, start)?;
        let width = inline_comment_code_width(source, line_start, line_end, None)?;
        Some(width + 1)
    }

    fn function_header_trailing_comment(&self, function: &FunctionDef) -> Option<(Span, String)> {
        if self.options().function_next_line() {
            return None;
        }

        let source = self.source();
        let header_end = function.header.span().end.offset;
        let body_start = stmt_span(function.body.as_ref()).start.offset;
        if header_end >= body_start || header_end >= source.len() {
            return None;
        }

        let line_end = source[header_end..body_start]
            .find('\n')
            .map(|offset| header_end + offset)
            .unwrap_or(body_start);
        let between = source.get(header_end..line_end)?;
        let comment_offset = between.find('#')?;
        if between[..comment_offset].contains('{') {
            return None;
        }
        let suffix_start = header_end;
        let comment = source
            .get(suffix_start..line_end)?
            .trim_end_matches([' ', '\t', '\r'])
            .to_string();
        (!comment.is_empty()).then(|| {
            (
                self.source_map()
                    .span_for_offsets(header_end + comment_offset, line_end),
                comment,
            )
        })
    }

    fn format_inline_stmts(&mut self, commands: &StmtSeq) -> Result<()> {
        for (index, stmt) in commands.iter().enumerate() {
            if index > 0 {
                if matches!(
                    commands[index - 1].terminator,
                    Some(StmtTerminator::Background(_))
                ) {
                    self.write_space();
                } else {
                    self.write_text("; ");
                }
            }
            self.format_inline_stmt(stmt)?;
        }
        Ok(())
    }

    fn format_inline_stmt(&mut self, stmt: &Stmt) -> Result<()> {
        if let Stmt {
            command: Command::Compound(CompoundCommand::Case(command)),
            negated: false,
            redirects,
            terminator: None,
            ..
        } = stmt
            && redirects.is_empty()
            && self.can_format_case_inline(command)
        {
            return self.format_inline_case(command);
        }

        if let Stmt {
            command: Command::Binary(command),
            negated: false,
            redirects,
            terminator: None,
            ..
        } = stmt
            && redirects.is_empty()
            && matches!(command.op, BinaryOp::And | BinaryOp::Or)
            && self.command_list_needs_inline_case(command)
        {
            return self.format_inline_command_list(command);
        }

        self.format_stmt(stmt)
    }

    fn command_list_needs_inline_case(&self, list: &BinaryCommand) -> bool {
        let mut rest = Vec::new();
        let first = collect_command_list_first(list, &mut rest);
        self.stmt_is_inline_case(first)
            || rest.iter().any(|item| self.stmt_is_inline_case(item.stmt))
    }

    fn stmt_is_inline_case(&self, stmt: &Stmt) -> bool {
        matches!(
            stmt,
            Stmt {
                command: Command::Compound(CompoundCommand::Case(command)),
                negated: false,
                redirects,
                terminator: None,
                ..
            } if redirects.is_empty() && self.can_format_case_inline(command)
        )
    }

    fn format_inline_command_list(&mut self, list: &BinaryCommand) -> Result<()> {
        let mut rest = Vec::new();
        let first = collect_command_list_first(list, &mut rest);
        self.format_inline_stmt(first)?;
        for item in &rest {
            if self
                .facts()
                .list_item_has_explicit_line_break(item.operator_span)
            {
                self.write_text(list_item_multiline_separator(item.operator));
                self.newline();
                self.with_indent(|formatter| {
                    let emitted_interstitial_comments = formatter
                        .emit_command_list_interstitial_comments(item.stmt, item.operator_span);
                    formatter.with_group_body_leading_filter(
                        emitted_interstitial_comments,
                        |formatter| formatter.format_inline_stmt(item.stmt),
                    )
                })?;
                continue;
            }
            self.write_text(list_item_inline_separator(item.operator));
            self.format_inline_stmt(item.stmt)?;
        }
        Ok(())
    }

    fn can_format_case_inline(&self, command: &CaseCommand) -> bool {
        command.cases.iter().all(|item| {
            item.body.is_empty()
                || item.body.len() == 1
                    && (self.facts().case_item_was_inline_in_source(item)
                        || case_item_pattern_body_terminator_was_inline_in_source(
                            item,
                            self.source(),
                        )
                        || case_item_body_was_inline_without_terminator(item))
                    && !self
                        .facts()
                        .sequence(&item.body, Some(command.span.end.offset))
                        .has_comments()
        })
    }

    fn format_inline_case(&mut self, command: &CaseCommand) -> Result<()> {
        let esac_span =
            last_shell_keyword_span(self.source(), self.source_map(), command.span, "esac");
        self.write_text("case ");
        self.write_word(&command.word);
        self.write_text(" in");
        for item in &command.cases {
            self.write_space();
            for (index, pattern) in item.patterns.iter().enumerate() {
                if index > 0 {
                    self.write_text(" | ");
                }
                self.write_pattern(pattern);
            }
            self.write_text(")");
            if item.body.is_empty() {
                self.write_space();
                self.write_text(case_terminator(item.terminator));
            } else {
                self.write_space();
                self.format_inline_stmts(&item.body)?;
                self.write_space();
                self.write_text(case_terminator(item.terminator));
            }
        }
        self.write_text(" esac");
        self.write_close_suffix_after_span(esac_span);
        Ok(())
    }

    fn then_separator_for_condition(&self, commands: &StmtSeq) -> &'static str {
        if self.inline_condition_ends_with_case(commands) {
            " then"
        } else {
            "; then"
        }
    }

    fn inline_condition_ends_with_case(&self, commands: &StmtSeq) -> bool {
        let [stmt] = commands.as_slice() else {
            return false;
        };
        matches!(
            stmt,
            Stmt {
                command: Command::Compound(CompoundCommand::Case(_)),
                negated: false,
                redirects,
                terminator: None,
                ..
            } if redirects.is_empty()
        )
    }

    fn format_body_with_upper_bound_and_open_blank(
        &mut self,
        commands: &StmtSeq,
        upper_bound: Option<usize>,
        preserve_open_blank: bool,
    ) -> Result<()> {
        self.format_body_with_upper_bound_open_blank_and_leading_filter(
            commands,
            upper_bound,
            preserve_open_blank,
            None,
        )
    }

    fn format_body_with_upper_bound_open_blank_and_leading_filter(
        &mut self,
        commands: &StmtSeq,
        upper_bound: Option<usize>,
        preserve_open_blank: bool,
        first_leading_min_offset: Option<usize>,
    ) -> Result<()> {
        if commands.is_empty() {
            return Ok(());
        }

        if self.options().compact_layout() {
            self.write_space();
            self.format_stmt_sequence_with_leading_filter(
                commands,
                upper_bound,
                first_leading_min_offset,
            )
        } else {
            self.newline();
            if preserve_open_blank {
                self.newline();
            }
            self.with_indent(|formatter| {
                formatter.format_stmt_sequence_with_leading_filter(
                    commands,
                    upper_bound,
                    first_leading_min_offset,
                )
            })
        }
    }

    fn finish_block(&mut self, close: &'static str) {
        if self.options().compact_layout() {
            self.write_text("; ");
            self.write_text(close);
        } else {
            self.newline();
            self.write_text(close);
        }
    }

    fn finish_block_with_close_suffix(&mut self, close: &'static str, close_span: Option<Span>) {
        self.finish_block(close);
        self.write_close_suffix_after_span(close_span);
    }

    fn write_close_suffix_after_span(&mut self, close_span: Option<Span>) {
        let Some(comment) = close_span.and_then(|span| self.close_suffix_comment_after_span(span))
        else {
            return;
        };
        let current_code_column = self.column.saturating_sub(self.line_indent_column);
        let padding = close_suffix_comment_padding(
            self.source(),
            &comment,
            current_code_column,
            self.line_indent_column,
        );
        for _ in 0..padding {
            self.write_space();
        }
        self.write_comment(&comment);
    }

    fn write_suffix_comment_after_span(&mut self, span: Span, nudge_aligned: bool) {
        let Some(comment) = self.suffix_comment_from_span(span) else {
            self.write_space();
            self.write_text(span.slice(self.source()).trim_start());
            return;
        };
        let current_code_column = self.column.saturating_sub(self.line_indent_column);
        let mut padding = trailing_comment_padding(self.source(), &comment, current_code_column);
        if nudge_aligned && trailing_comment_alignment_column(self.source(), &comment).is_some() {
            padding += 1;
        }
        for _ in 0..padding {
            self.write_space();
        }
        self.write_comment(&comment);
    }

    fn suffix_comment_from_span(&self, span: Span) -> Option<SourceComment<'source>> {
        let source = self.source();
        let raw = span.slice(source);
        let leading_padding = raw.len() - raw.trim_start_matches([' ', '\t']).len();
        let comment = raw[leading_padding..].trim_end_matches([' ', '\t', '\r']);
        if !comment.starts_with('#') {
            return None;
        }
        let absolute_start = span.start.offset + leading_padding;
        let absolute_end = absolute_start + comment.len();
        self.source_map()
            .source_comment_for_offsets(absolute_start, absolute_end)
    }

    fn close_suffix_comment_after_span(&self, close_span: Span) -> Option<SourceComment<'source>> {
        if close_span.start.line != close_span.end.line {
            return None;
        }
        let source = self.source();
        let start = close_span.end.offset.min(source.len());
        let suffix_source = source.get(start..)?;
        let line_end = suffix_source
            .find('\n')
            .map_or(source.len(), |offset| start + offset);
        let suffix = source.get(start..line_end)?;
        let mut comment_start = None;
        for (offset, ch) in suffix.char_indices() {
            match ch {
                ' ' | '\t' => {}
                '#' => {
                    comment_start = Some(start + offset);
                    break;
                }
                _ => return None,
            }
        }
        let comment_start = comment_start?;
        let comment_end = source
            .get(comment_start..line_end)?
            .trim_end_matches([' ', '\t', '\r'])
            .len()
            + comment_start;
        self.source_map()
            .source_comment_for_offsets(comment_start, comment_end)
    }

    fn format_group_with_upper_bound(
        &mut self,
        open: &'static str,
        close: &'static str,
        open_char: char,
        commands: &StmtSeq,
        leading_space: bool,
        upper_bound: Option<usize>,
    ) -> Result<()> {
        let filter_leading_before_open = self.filter_next_group_body_leading_before_open;
        self.filter_next_group_body_leading_before_open = false;
        if leading_space {
            self.write_space();
        }
        self.write_text(open);
        let open_suffix_span = self
            .facts()
            .sequence(commands, upper_bound)
            .group_open_suffix_span();
        if let Some(span) = open_suffix_span {
            self.write_suffix_comment_after_span(span, true);
        }
        let source = self.source();
        let group_span = group_attachment_span(
            commands.as_slice(),
            self.source_map(),
            open_char,
            matching_group_close_char(open_char),
        );
        let open_end_offset = open_suffix_span
            .map(|span| span.end.offset)
            .or_else(|| group_span.map(|span| span.start.offset.saturating_add(open.len())));
        let preserve_open_blank = open_end_offset.is_some_and(|offset| {
            body_has_blank_line_after_open(source, self.source_map(), offset, commands)
        });
        let close_char = matching_group_close_char(open_char);
        let preserve_close_blank = group_span.is_some_and(|span| {
            let close_offset =
                group_close_offset(source, span, upper_bound, close_char, close.len());
            source_has_blank_line_immediately_before_offset(source, close_offset)
        });

        self.format_body_with_upper_bound_open_blank_and_leading_filter(
            commands,
            upper_bound,
            preserve_open_blank,
            filter_leading_before_open
                .then_some(open_end_offset)
                .flatten(),
        )?;
        if preserve_close_blank {
            self.newline();
        }
        self.finish_block(close);
        Ok(())
    }

    fn format_redirect_list(&mut self, redirects: &[Redirect]) {
        let source = self.source();
        let mut index = 0;
        let mut wrote_redirect = false;
        while index < redirects.len() {
            if wrote_redirect {
                self.write_space();
            }
            if append_both_redirect_matches_source(redirects, index, source) {
                self.format_append_both_redirect(&redirects[index]);
                index += 2;
                wrote_redirect = true;
                continue;
            }
            let redirect = &redirects[index];
            self.format_redirect(redirect);
            index += 1;
            wrote_redirect = true;
        }
    }

    fn format_redirect(&mut self, redirect: &Redirect) {
        let source = self.source();
        let options = self.options().clone();
        if !options.simplify()
            && !options.minify()
            && let Some(raw) = raw_redirect_source_slice(redirect, source)
            && should_preserve_raw_redirect(raw)
        {
            self.write_text(raw);
            return;
        }

        if let Some(name) = &redirect.fd_var {
            self.write_text("{");
            self.write_text(name.as_str());
            self.write_text("}");
        } else if let Some(fd) = redirect.fd
            && (should_render_explicit_fd(fd, redirect.kind)
                || redirect_source_has_explicit_fd(redirect, source, fd))
        {
            self.write_display(fd);
        }

        self.write_text(match redirect.kind {
            RedirectKind::Output => ">",
            RedirectKind::Clobber => ">|",
            RedirectKind::Append => ">>",
            RedirectKind::Input => "<",
            RedirectKind::ReadWrite => "<>",
            RedirectKind::HereDoc => "<<",
            RedirectKind::HereDocStrip => "<<-",
            RedirectKind::HereString => "<<<",
            RedirectKind::DupOutput => ">&",
            RedirectKind::DupInput => "<&",
            RedirectKind::OutputBoth => "&>",
        });

        let mut target = self.take_scratch_buffer();
        let source_map = self.source_map().clone();
        {
            let facts = self.facts();
            match (redirect.word_target(), redirect.heredoc()) {
                (Some(word), None) => render_word_syntax_with_facts_to_buf(
                    word,
                    source,
                    &options,
                    &source_map,
                    facts,
                    &mut target,
                ),
                (None, Some(heredoc)) => render_word_syntax_with_facts_to_buf(
                    &heredoc.delimiter.raw,
                    source,
                    &options,
                    &source_map,
                    facts,
                    &mut target,
                ),
                (None, None) => {}
                (Some(_), Some(_)) => {
                    unreachable!("redirect target cannot be both word and heredoc")
                }
            }
        }
        if redirect_target_starts_on_continuation_line(redirect, source) {
            self.line_continuation();
            self.write_indent_units(1);
        } else if needs_space_before_target(
            redirect.kind,
            normalized_redirect_target(redirect.kind, &target),
            options.space_redirects(),
        ) {
            self.write_space();
        }
        self.write_rendered_shell_text(normalized_redirect_target(redirect.kind, &target));
        self.restore_scratch_buffer(target);
    }

    fn format_append_both_redirect(&mut self, redirect: &Redirect) {
        let source = self.source();
        let options = self.options().clone();
        self.write_text("&>>");

        let mut target = self.take_scratch_buffer();
        let source_map = self.source_map().clone();
        {
            let facts = self.facts();
            match (redirect.word_target(), redirect.heredoc()) {
                (Some(word), None) => render_word_syntax_with_facts_to_buf(
                    word,
                    source,
                    &options,
                    &source_map,
                    facts,
                    &mut target,
                ),
                (None, Some(heredoc)) => render_word_syntax_with_facts_to_buf(
                    &heredoc.delimiter.raw,
                    source,
                    &options,
                    &source_map,
                    facts,
                    &mut target,
                ),
                (None, None) => {}
                (Some(_), Some(_)) => {
                    unreachable!("redirect target cannot be both word and heredoc")
                }
            }
        }
        let normalized_target = normalized_redirect_target(redirect.kind, &target);
        if redirect_target_starts_on_continuation_line(redirect, source) {
            self.line_continuation();
            self.write_indent_units(1);
        } else if needs_space_before_target(
            redirect.kind,
            normalized_target,
            options.space_redirects(),
        ) {
            self.write_space();
        }
        self.write_rendered_shell_text(normalized_target);
        self.restore_scratch_buffer(target);
    }

    fn queue_heredocs(&mut self, redirects: &[Redirect]) {
        let source = self.source();
        for redirect in redirects {
            let Some(heredoc) = redirect.heredoc() else {
                continue;
            };
            let body = if !self.options.simplify()
                && !self.options.minify()
                && !heredoc_body_contains_command_substitution(&heredoc.body)
                && heredoc.body.span.end.offset <= source.len()
                && heredoc.body.span.start.offset <= heredoc.body.span.end.offset
            {
                heredoc.body.span.slice(source).to_owned()
            } else {
                let mut rendered = String::new();
                render_heredoc_body_to_buf(
                    &heredoc.body,
                    source,
                    &self.options,
                    self.facts,
                    self.indent_level.saturating_add(1),
                    &mut rendered,
                );
                rendered
            };
            // The opening redirection keeps delimiter quoting, but the closing
            // marker line uses the cooked delimiter text after quote removal.
            let delimiter = heredoc_closing_marker_source(heredoc, source)
                .unwrap_or_else(|| heredoc.delimiter.cooked.to_string());
            self.pending_heredocs.push(PendingHeredoc {
                body,
                delimiter,
                strip_tabs: matches!(redirect.kind, RedirectKind::HereDocStrip),
            });
        }
    }

    fn format_standalone_multiline_compound_assignment(
        &mut self,
        assignment: &shuck_ast::Assignment,
    ) -> Result<()> {
        let source = self.source();
        if compound_assignment_is_single_case_command_substitution(assignment) {
            self.write_assignment(assignment);
            return Ok(());
        }

        if assignment_has_multiline_literal_source(assignment, source)
            && !Self::compound_assignment_source_has_line_continuations(
                assignment.span.slice(source),
            )
        {
            self.write_multiline_compound_literal_assignment(assignment);
            return Ok(());
        }

        let Some(layout) = multiline_compound_assignment_layout(assignment, source) else {
            self.write_assignment(assignment);
            return Ok(());
        };

        self.write_assignment_head(assignment);
        self.write_text("(");
        self.write_standalone_multiline_compound_assignment_layout(&layout);
        Ok(())
    }

    fn compound_assignment_source_has_line_continuations(raw: &str) -> bool {
        raw.contains("\\\n") || raw.contains("\\\r\n")
    }

    fn write_multiline_compound_literal_assignment(&mut self, assignment: &Assignment) {
        let raw = assignment.span.slice(self.source());
        let Some((head, tail)) = raw.split_once('\n') else {
            self.write_text(raw);
            return;
        };

        self.write_text(head);
        let mut quote = None;
        for line in tail.lines() {
            self.newline();
            if quote.is_some() {
                self.write_verbatim(line.trim_end_matches('\r'));
                quote = multiline_literal_quote_state_after_line(line, quote);
                continue;
            }

            let trimmed = line.trim_start_matches([' ', '\t']).trim_end_matches('\r');
            if trimmed.starts_with(')') {
                self.write_text(trimmed);
            } else {
                self.write_indent_units(1);
                self.write_text(trimmed);
            }
            quote = multiline_literal_quote_state_after_line(trimmed, quote);
        }
    }

    fn can_inline_body(&self, commands: &StmtSeq, enclosing_span: Span) -> bool {
        self.can_inline_body_with_upper_bound(
            commands,
            enclosing_span,
            Some(enclosing_span.end.offset),
        )
    }

    fn can_inline_body_with_upper_bound(
        &self,
        commands: &StmtSeq,
        enclosing_span: Span,
        upper_bound: Option<usize>,
    ) -> bool {
        let [command] = commands.as_slice() else {
            return false;
        };
        if matches!(command.terminator, Some(StmtTerminator::Background(_)))
            || !self.can_inline_stmt(command)
        {
            return false;
        }

        if self.facts().sequence(commands, upper_bound).has_comments() {
            return false;
        }

        self.options().compact_layout()
            || stmt_span(command).start.line == enclosing_span.start.line
    }

    fn can_inline_group(&self, commands: &StmtSeq, open_char: char) -> bool {
        let [command] = commands.as_slice() else {
            return false;
        };

        self.can_inline_stmt(command)
            && self.can_inline_body(commands, stmt_span(command))
            && (stmt_span(command).start.line == stmt_span(command).end.line
                || self.group_delimiters_attach_to_wrapped_body(commands, open_char))
    }

    fn group_has_inline_source_shape(&self, commands: &StmtSeq, open_char: char) -> bool {
        self.facts().group_was_inline_in_source(commands)
            || self.group_delimiters_attach_to_wrapped_body(commands, open_char)
    }

    fn group_delimiters_attach_to_wrapped_body(&self, commands: &StmtSeq, open_char: char) -> bool {
        let (Some(first), Some(last)) = (commands.first(), commands.last()) else {
            return false;
        };
        let Some(group_span) = group_attachment_span(
            commands.as_slice(),
            self.source_map(),
            open_char,
            matching_group_close_char(open_char),
        ) else {
            return false;
        };

        group_span.start.line == stmt_format_span(first).start.line
            && group_span.end.line == stmt_format_span(last).end.line
    }

    fn can_inline_source_line_subshell(
        &self,
        commands: &StmtSeq,
        upper_bound: Option<usize>,
    ) -> bool {
        let [stmt] = commands.as_slice() else {
            return false;
        };
        if self.facts().sequence(commands, upper_bound).has_comments()
            || self.facts().stmt(stmt).preserve_verbatim()
            || self.facts().stmt(stmt).has_trailing_comment()
        {
            return false;
        }
        if commands.span.start.line != commands.span.end.line {
            return false;
        }

        true
    }

    fn can_format_multiline_subshell_inline(
        &self,
        commands: &StmtSeq,
        upper_bound: Option<usize>,
    ) -> bool {
        let [stmt] = commands.as_slice() else {
            return false;
        };
        if self
            .facts()
            .sequence(commands, upper_bound)
            .group_open_suffix_span()
            .is_some()
            || self.facts().sequence(commands, upper_bound).has_comments()
        {
            return false;
        }
        let Some(group_span) =
            group_attachment_span(commands.as_slice(), self.source_map(), '(', ')')
        else {
            return false;
        };
        let group_source = group_span.slice(self.source());
        if !group_source.contains('\n')
            || group_source.contains("\\\n")
            || group_source.contains("\\\r\n")
        {
            return false;
        }

        let source = self.source();
        let first_start = stmt_span(stmt).start.offset.min(source.len());
        let open_end = group_span.start.offset.saturating_add('('.len_utf8());
        if source
            .get(open_end..first_start)
            .is_none_or(|between| between.contains('\n'))
        {
            return false;
        }

        let close_offset = group_close_offset(source, group_span, upper_bound, ')', ')'.len_utf8());
        let stmt_end = stmt_span(stmt)
            .end
            .offset
            .min(close_offset)
            .min(source.len());
        source
            .get(stmt_end..close_offset)
            .is_some_and(|between| !between.contains('\n'))
    }

    fn can_inline_stmt(&self, stmt: &Stmt) -> bool {
        let stmt_facts = self.facts().stmt(stmt);
        if stmt_facts.preserve_verbatim() || stmt_facts.has_trailing_comment() {
            return false;
        }

        matches!(
            &stmt.command,
            Command::Simple(_)
                | Command::Builtin(_)
                | Command::Decl(_)
                | Command::Function(_)
                | Command::Binary(_)
                | Command::Compound(
                    CompoundCommand::Conditional(_)
                        | CompoundCommand::Arithmetic(_)
                        | CompoundCommand::Time(_)
                )
        )
    }

    fn format_conditional_expr(&mut self, expression: &ConditionalExpr) -> Result<()> {
        match expression {
            ConditionalExpr::Binary(expr) => self.format_conditional_binary(expr),
            ConditionalExpr::Unary(expr) => self.format_conditional_unary(expr),
            ConditionalExpr::Parenthesized(expr) => self.format_conditional_paren(expr),
            ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
                self.write_word(word);
                Ok(())
            }
            ConditionalExpr::Pattern(pattern) => {
                self.write_pattern(pattern);
                Ok(())
            }
            ConditionalExpr::VarRef(reference) => {
                self.write_var_ref(reference);
                Ok(())
            }
        }
    }

    fn format_conditional_binary(&mut self, expression: &ConditionalBinaryExpr) -> Result<()> {
        self.format_conditional_expr(&expression.left)?;
        self.write_space();
        self.write_text(expression.op.as_str());
        if conditional_binary_has_explicit_rhs_break(expression, self.source()) {
            self.newline();
            if !conditional_expr_contains_command_substitution(&expression.left) {
                self.write_indent_units(1);
            }
            self.format_conditional_expr(&expression.right)?;
            return Ok(());
        }
        self.write_space();
        if matches!(expression.op, ConditionalBinaryOp::RegexMatch) {
            self.write_conditional_regex_rhs(&expression.right)
        } else {
            self.format_conditional_expr(&expression.right)
        }
    }

    fn write_conditional_regex_rhs(&mut self, expression: &ConditionalExpr) -> Result<()> {
        let raw = expression.span().slice(self.source()).trim();
        if raw.contains('\n') {
            self.format_conditional_expr(expression)
        } else {
            self.write_text(raw);
            Ok(())
        }
    }

    fn format_conditional_unary(&mut self, expression: &ConditionalUnaryExpr) -> Result<()> {
        self.write_text(expression.op.as_str());
        self.write_space();
        self.format_conditional_expr(&expression.expr)
    }

    fn format_conditional_paren(&mut self, expression: &ConditionalParenExpr) -> Result<()> {
        self.write_text("(");
        self.format_conditional_expr(&expression.expr)?;
        self.write_text(")");
        Ok(())
    }

    fn conditional_needs_tight_close(&mut self, expression: &ConditionalExpr) -> bool {
        match expression {
            ConditionalExpr::Word(word) => self.conditional_word_needs_tight_close(word),
            ConditionalExpr::Unary(expression)
                if matches!(expression.op, ConditionalUnaryOp::Not) =>
            {
                self.conditional_needs_tight_close(&expression.expr)
            }
            _ => false,
        }
    }

    fn conditional_word_needs_tight_close(&mut self, word: &Word) -> bool {
        let source_map = self.source_map().clone();
        let mut rendered = self.take_scratch_buffer();
        {
            let facts = self.facts();
            render_word_syntax_with_facts_to_buf(
                word,
                self.source(),
                self.options(),
                &source_map,
                facts,
                &mut rendered,
            );
        }
        let needs_tight_close = matches!(
            rendered.as_str(),
            "!" | "-a"
                | "-b"
                | "-c"
                | "-d"
                | "-e"
                | "-f"
                | "-g"
                | "-G"
                | "-h"
                | "-k"
                | "-L"
                | "-N"
                | "-n"
                | "-o"
                | "-O"
                | "-p"
                | "-r"
                | "-R"
                | "-s"
                | "-S"
                | "-t"
                | "-u"
                | "-v"
                | "-w"
                | "-x"
                | "-z"
        );
        self.restore_scratch_buffer(rendered);
        needs_tight_close
    }

    fn write_case_prefix(&mut self, levels: usize) {
        if levels == 0 {
            return;
        }
        self.write_indent_units(levels);
    }
}

fn heredoc_body_needs_separator(body: &str) -> bool {
    !body.is_empty() && !body.ends_with('\n') && !body.ends_with('\r')
}

fn minimum_leading_tabs_in_non_empty_lines(text: &str) -> usize {
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|line| line.bytes().take_while(|byte| *byte == b'\t').count())
        .min()
        .unwrap_or(0)
}

fn assignment_contains_command_heredoc(assignment: &Assignment) -> bool {
    match &assignment.value {
        AssignmentValue::Scalar(word) => word_contains_command_heredoc(word),
        AssignmentValue::Compound(array) => array.elements.iter().any(|element| match element {
            ArrayElem::Sequential(word)
            | ArrayElem::Keyed { value: word, .. }
            | ArrayElem::KeyedAppend { value: word, .. } => word_contains_command_heredoc(word),
        }),
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
    stmt_seq_is_single_case_command(body)
}

fn stmt_seq_is_single_case_command(body: &StmtSeq) -> bool {
    let [stmt] = body.as_slice() else {
        return false;
    };
    !stmt.negated
        && stmt.redirects.is_empty()
        && matches!(&stmt.command, Command::Compound(CompoundCommand::Case(_)))
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
            stmt_seq_has_heredoc(body)
        }
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter()
            .any(|part| word_part_contains_command_heredoc(&part.kind)),
        _ => false,
    }
}

#[derive(Debug, Clone)]
struct RenderedHeredocTail {
    delimiter: String,
    strip_tabs: bool,
}

impl RenderedHeredocTail {
    fn closes(&self, line: &str) -> bool {
        if self.strip_tabs {
            line.trim_start_matches('\t') == self.delimiter
        } else {
            line == self.delimiter
        }
    }
}

fn rendered_shell_text_has_heredoc_tail(text: &str) -> bool {
    text.lines()
        .any(|line| rendered_heredoc_tail_start(line).is_some())
}

fn rendered_heredoc_tail_start(line: &str) -> Option<RenderedHeredocTail> {
    let marker = line.find("<<")?;
    let after_marker = &line[marker + 2..];
    if after_marker.starts_with('<') {
        return None;
    }
    let (strip_tabs, after_marker) = if let Some(rest) = after_marker.strip_prefix('-') {
        (true, rest)
    } else {
        (false, after_marker)
    };
    let delimiter = after_marker
        .trim_start()
        .split_whitespace()
        .next()?
        .trim_matches(['\'', '"'])
        .to_string();
    (!delimiter.is_empty()).then_some(RenderedHeredocTail {
        delimiter,
        strip_tabs,
    })
}

fn normalize_rendered_heredoc_start_spacing(line: &str) -> Option<String> {
    let marker = line.find("<<")?;
    let after_marker = &line[marker + 2..];
    if after_marker.starts_with('<') {
        return None;
    }
    let operator_end = marker + if after_marker.starts_with('-') { 3 } else { 2 };
    let target_start = line[operator_end..]
        .char_indices()
        .find_map(|(index, ch)| (!matches!(ch, ' ' | '\t' | '\r')).then_some(operator_end + index))
        .unwrap_or(line.len());
    if target_start == operator_end || target_start == line.len() {
        return None;
    }

    let mut normalized = String::with_capacity(line.len());
    normalized.push_str(&line[..operator_end]);
    normalized.push_str(&line[target_start..]);
    Some(normalized)
}

fn heredoc_closing_marker_source(heredoc: &Heredoc, source: &str) -> Option<String> {
    let (start, line_end) = heredoc_closing_marker_bounds(heredoc, source)?;
    let line = source.get(start..line_end)?;
    (line.trim_start_matches('\t') == heredoc.delimiter.cooked.as_str()).then(|| line.to_string())
}

fn heredoc_closing_marker_bounds(heredoc: &Heredoc, source: &str) -> Option<(usize, usize)> {
    let mut start = heredoc.body.span.end.offset.min(source.len());
    if source
        .as_bytes()
        .get(start)
        .is_some_and(|byte| *byte == b'\n')
    {
        start += 1;
    }
    let line_end = source[start..]
        .find(['\n', '\r'])
        .map_or(source.len(), |offset| start + offset);
    let line = source.get(start..line_end)?;
    (line.trim_start_matches('\t') == heredoc.delimiter.cooked.as_str())
        .then_some((start, line_end))
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

fn append_both_redirect_matches_source(redirects: &[Redirect], index: usize, source: &str) -> bool {
    let Some(redirect) = redirects.get(index) else {
        return false;
    };
    let Some(next) = redirects.get(index + 1) else {
        return false;
    };
    append_both_redirect_pair_matches_source(redirect, next, source)
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
        || !next
            .word_target()
            .and_then(|word| word.try_static_text(source))
            .is_some_and(|target| target == "1")
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

fn redirect_target_starts_on_continuation_line(redirect: &Redirect, source: &str) -> bool {
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
    source
        .get(redirect.span.start.offset.min(source.len())..target_start.min(source.len()))
        .is_some_and(|between| between.contains("\\\n") || between.contains("\\\r\n"))
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

fn redirect_list_needs_leading_space(
    command_span: Span,
    redirects: &[Redirect],
    source: &str,
) -> bool {
    redirects.first().is_none_or(|redirect| {
        !redirect_is_attached_process_substitution(command_span, redirect, source)
    })
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

fn decl_operand_span(operand: &DeclOperand) -> Span {
    match operand {
        DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => word.span,
        DeclOperand::Name(name) => name.span,
        DeclOperand::Assignment(assignment) => assignment.span,
    }
}

fn assignment_has_multiline_literal_source(assignment: &Assignment, source: &str) -> bool {
    match &assignment.value {
        AssignmentValue::Scalar(word) => word_has_multiline_literal_source(word, source),
        AssignmentValue::Compound(array) => array.elements.iter().any(|element| match element {
            ArrayElem::Sequential(word)
            | ArrayElem::Keyed { value: word, .. }
            | ArrayElem::KeyedAppend { value: word, .. } => {
                word_has_multiline_literal_source(word, source)
            }
        }),
    }
}

fn normalize_scalar_assignment_unquoted_continuations(
    assignment: &Assignment,
    source: &str,
) -> Option<String> {
    if assignment_source_has_command_substitution(assignment, source) {
        return None;
    }
    let AssignmentValue::Scalar(_) = &assignment.value else {
        return None;
    };
    let raw = assignment.span.slice(source);
    if !raw.contains("\\\n") && !raw.contains("\\\r\n") {
        return None;
    }

    let mut head = String::new();
    render_assignment_head_to_buf(assignment, source, &mut head);
    let raw_value = raw.strip_prefix(&head)?;
    let normalized_value = normalize_raw_unquoted_word_continuations(raw_value)?;
    let mut normalized = head;
    normalized.push_str(&normalized_value);
    Some(normalized)
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
        && raw_backslash_continuation_is_quoted(raw)
        && !raw.contains("$(")
        && !raw.contains('`')
        && !raw.contains("<(")
        && !raw.contains(">(")
}

fn raw_backslash_continuation_is_quoted(raw: &str) -> bool {
    let mut chars = raw.chars().peekable();
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    while let Some(ch) = chars.next() {
        if ch == '\'' && !in_double_quotes {
            in_single_quotes = !in_single_quotes;
            continue;
        }
        if ch == '"' && !in_single_quotes {
            in_double_quotes = !in_double_quotes;
            continue;
        }
        if ch == '\\' {
            let mut probe = chars.clone();
            let escaped_newline = match probe.next() {
                Some('\n') => true,
                Some('\r') => probe.next().is_some_and(|next| next == '\n'),
                _ => false,
            };
            if escaped_newline {
                return in_single_quotes || in_double_quotes;
            }
            if !in_single_quotes {
                chars.next();
            }
        }
    }
    false
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
    source: &str,
) -> bool {
    match &assignment.value {
        AssignmentValue::Scalar(word) => {
            word_is_quoted_formattable_command_substitution_only(word, source)
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

fn stmt_semicolon_terminator_starts_on_continuation_line(stmt: &Stmt, source: &str) -> bool {
    let Some(terminator_span) = stmt.terminator_span else {
        return false;
    };
    let render_end = stmt
        .redirects
        .last()
        .map(|redirect| redirect.span.end.offset)
        .unwrap_or_else(|| command_format_span(&stmt.command).end.offset);
    has_newline_between_offsets(source, render_end, terminator_span.start.offset)
}

fn stmt_rendered_end_line_after_format(
    stmt: &Stmt,
    source: &str,
    source_map: &SourceMap<'_>,
    fallback: usize,
) -> usize {
    if matches!(stmt.terminator, Some(StmtTerminator::Semicolon))
        && stmt_semicolon_terminator_starts_on_continuation_line(stmt, source)
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
        Command::Compound(CompoundCommand::BraceGroup(commands))
            if stmt.redirects.is_empty() && stmt.terminator.is_none() =>
        {
            if let Some(span) = group_attachment_span(commands.as_slice(), source_map, '{', '}') {
                let close_offset = group_close_offset(
                    source,
                    span,
                    Some(stmt_span(stmt).end.offset),
                    '}',
                    '}'.len_utf8(),
                );
                return source_map.line_number_for_offset(close_offset);
            }
        }
        Command::Compound(CompoundCommand::Subshell(commands))
            if stmt.redirects.is_empty() && stmt.terminator.is_none() =>
        {
            if let Some(span) = group_attachment_span(commands.as_slice(), source_map, '(', ')') {
                let close_offset = group_close_offset(
                    source,
                    span,
                    Some(stmt_span(stmt).end.offset),
                    ')',
                    ')'.len_utf8(),
                );
                return source_map.line_number_for_offset(close_offset);
            }
        }
        _ => {}
    }
    fallback
}

fn if_condition_starts_after_keyword(command: &IfCommand) -> bool {
    command
        .condition
        .first()
        .is_some_and(|stmt| stmt_span(stmt).start.line > command.span.start.line)
}

fn if_condition_has_explicit_statement_break(
    command: &IfCommand,
    then_span: Span,
    source: &str,
) -> bool {
    condition_sequence_has_explicit_statement_break(
        &command.condition,
        then_span.start.offset,
        source,
    )
}

fn condition_sequence_has_explicit_statement_break(
    condition: &StmtSeq,
    upper_bound: usize,
    source: &str,
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
        let end = upper_bound.min(source.len());
        return source
            .get(start..command_end)
            .is_some_and(has_unescaped_line_break)
            || source
                .get(command_end..end)
                .is_some_and(|separator| separator.contains('#'));
    }

    condition.as_slice().windows(2).any(|pair| {
        let previous_start = stmt_span(&pair[0]).start.offset;
        let next_start = stmt_span(&pair[1]).start.offset;
        has_newline_between_offsets(source, previous_start, next_start)
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
) -> bool {
    let upper_bound =
        branch_open_keyword_start(body, source, "then").unwrap_or(body.span.start.offset);
    condition_sequence_has_explicit_statement_break(condition, upper_bound, source)
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
    keyword: &str,
) -> bool {
    let Some(first) = condition.first() else {
        return false;
    };
    let Some((mut line_start, _)) = line_bounds_for_offset(source, stmt_span(first).start.offset)
    else {
        return false;
    };

    while let Some((start, end)) = previous_line_bounds(source, line_start) {
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

fn branch_open_keyword_start(sequence: &StmtSeq, source: &str, keyword: &str) -> Option<usize> {
    let first = sequence.first()?;
    let first_start = stmt_span(first).start.offset;
    let mut search_end = first_start.min(source.len());
    loop {
        let offset = source[..search_end].rfind(keyword)?;
        let end = offset + keyword.len();
        if shell_keyword_boundaries_match(source, offset, end)
            && !line_has_shell_comment_before(source, offset)
        {
            return Some(offset);
        }
        search_end = offset;
    }
}

fn line_has_shell_comment_before(source: &str, offset: usize) -> bool {
    let upper = offset.min(source.len());
    let line_start = source[..upper]
        .rfind('\n')
        .map_or(0, |newline| newline.saturating_add(1));
    let mut cursor = line_start;
    while cursor < upper {
        let Some(ch) = source[cursor..].chars().next() else {
            break;
        };
        match ch {
            '\'' => {
                cursor = skip_single_quoted(source, cursor + ch.len_utf8(), upper);
            }
            '"' => {
                cursor = skip_double_quoted(source, cursor + ch.len_utf8(), upper);
            }
            '#' if shell_comment_can_start(source, cursor) => return true,
            _ => cursor += ch.len_utf8(),
        }
    }
    false
}

fn raw_grouped_if_condition<'a>(
    command: &IfCommand,
    then_span: Span,
    source: &'a str,
) -> Option<String> {
    if !if_condition_starts_after_keyword(command) {
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
    let outer_indent = line_indent_before_offset(source, command.span.start.offset).unwrap_or("");
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

fn body_has_blank_line_after_open(
    source: &str,
    source_map: &SourceMap<'_>,
    open_end_offset: usize,
    commands: &StmtSeq,
) -> bool {
    let Some(mut first_start) = sequence_first_content_offset(commands, source, source_map) else {
        return false;
    };
    if first_start <= open_end_offset
        && let Some(stmt) = commands.first()
    {
        first_start = stmt_first_content_offset(stmt, source, source_map);
    }
    if source_map.line_number_for_offset(first_start)
        == source_map.line_number_for_offset(open_end_offset)
        && let Some(stmt) = commands.first()
    {
        first_start = stmt_first_content_offset(stmt, source, source_map);
    }
    let first_start = source_map
        .first_comment_between(open_end_offset, first_start)
        .filter(|comment_start| {
            source_map.line_number_for_offset(*comment_start)
                != source_map.line_number_for_offset(open_end_offset)
        })
        .unwrap_or(first_start);
    gap_has_blank_line(source, open_end_offset, first_start)
        || (source
            .get(..open_end_offset.min(source.len()))
            .is_some_and(|prefix| prefix.ends_with('\n'))
            && gap_starts_with_empty_physical_line(source, open_end_offset, first_start))
}

fn body_has_blank_line_after_keyword(
    source: &str,
    source_map: &SourceMap<'_>,
    search_start: usize,
    keyword: &str,
    commands: &StmtSeq,
) -> bool {
    let Some(first_start) = sequence_first_content_offset(commands, source, source_map) else {
        return false;
    };
    let Some(prefix) = source.get(search_start.min(source.len())..first_start.min(source.len()))
    else {
        return false;
    };
    let Some(keyword_end) = last_shell_keyword_end(prefix, keyword) else {
        return false;
    };
    gap_has_blank_line(source, search_start + keyword_end, first_start)
}

fn source_has_blank_line_immediately_before_offset(source: &str, offset: usize) -> bool {
    let offset = offset.min(source.len());
    let close_line_start = source[..offset]
        .rfind('\n')
        .map_or(0, |index| index.saturating_add(1));
    if !source[close_line_start..offset]
        .trim_matches([' ', '\t', '\r'])
        .is_empty()
    {
        return false;
    }
    if close_line_start == 0 {
        return false;
    }
    let previous_line_end = close_line_start.saturating_sub(1);
    let previous_line_start = source[..previous_line_end]
        .rfind('\n')
        .map_or(0, |index| index.saturating_add(1));
    source[previous_line_start..previous_line_end]
        .trim_matches([' ', '\t', '\r'])
        .is_empty()
}

fn source_has_blank_line_before_last_keyword(
    source: &str,
    source_map: &SourceMap<'_>,
    span: Span,
    keyword: &str,
) -> bool {
    last_shell_keyword_span(source, source_map, span, keyword).is_some_and(|keyword_span| {
        source_has_blank_line_immediately_before_offset(source, keyword_span.start.offset)
    })
}

fn source_has_blank_line_before_last_keyword_after(
    source: &str,
    start_offset: usize,
    span: Span,
    keyword: &str,
) -> bool {
    let upper = span.end.offset.min(source.len());
    let lower = start_offset.max(span.start.offset).min(upper);
    let Some(slice) = source.get(lower..upper) else {
        return false;
    };
    let Some(keyword_start) = slice
        .match_indices(keyword)
        .filter_map(|(start, _)| {
            let end = start + keyword.len();
            shell_keyword_boundaries_match(slice, start, end).then_some(start)
        })
        .last()
    else {
        return false;
    };
    gap_has_empty_physical_line(source, lower, lower + keyword_start)
}

fn sequence_first_content_offset(
    commands: &StmtSeq,
    source: &str,
    source_map: &SourceMap<'_>,
) -> Option<usize> {
    let mut first = commands
        .leading_comments
        .iter()
        .map(|comment| usize::from(comment.range.start()))
        .min();
    if let Some(stmt) = commands.first() {
        first = first
            .into_iter()
            .chain(
                stmt.leading_comments
                    .iter()
                    .map(|comment| usize::from(comment.range.start())),
            )
            .chain(std::iter::once(stmt_first_content_offset(
                stmt, source, source_map,
            )))
            .min();
    }
    first
}

fn stmt_first_content_offset(stmt: &Stmt, source: &str, source_map: &SourceMap<'_>) -> usize {
    match &stmt.command {
        Command::Binary(command) => stmt_first_content_offset(&command.left, source, source_map),
        Command::Compound(CompoundCommand::BraceGroup(commands)) => {
            group_attachment_span(commands.as_slice(), source_map, '{', '}')
                .map(|span| span.start.offset)
                .unwrap_or_else(|| {
                    stmt_verbatim_span_with_source_map(stmt, source_map)
                        .start
                        .offset
                })
        }
        Command::Compound(CompoundCommand::Subshell(commands)) => {
            group_attachment_span(commands.as_slice(), source_map, '(', ')')
                .map(|span| span.start.offset)
                .unwrap_or_else(|| {
                    stmt_verbatim_span_with_source_map(stmt, source_map)
                        .start
                        .offset
                })
        }
        _ => {
            stmt_verbatim_span_with_source_map(stmt, source_map)
                .start
                .offset
        }
    }
}

fn last_shell_keyword_end(text: &str, keyword: &str) -> Option<usize> {
    text.match_indices(keyword)
        .filter_map(|(start, _)| {
            let end = start + keyword.len();
            shell_keyword_boundaries_match(text, start, end).then_some(end)
        })
        .last()
}

fn last_shell_keyword_span(
    source: &str,
    source_map: &SourceMap<'_>,
    span: Span,
    keyword: &str,
) -> Option<Span> {
    let upper = span.end.offset.min(source.len());
    let lower = span.start.offset.min(upper);
    let slice = source.get(lower..upper)?;
    let start = slice
        .match_indices(keyword)
        .filter_map(|(start, _)| {
            let end = start + keyword.len();
            shell_keyword_boundaries_match(slice, start, end).then_some(lower + start)
        })
        .last()?;
    Some(source_map.span_for_offsets(start, start + keyword.len()))
}

fn if_close_span(source: &str, source_map: &SourceMap<'_>, command: &IfCommand) -> Option<Span> {
    let (syntax_close, keyword) = match command.syntax {
        IfSyntax::ThenFi { fi_span, .. } => (fi_span, "fi"),
        IfSyntax::Brace {
            right_brace_span, ..
        } => (right_brace_span, "}"),
    };
    let syntax_close = normalized_close_keyword_span(source, source_map, syntax_close, keyword);
    matching_if_close_span(source, source_map, command.span).or(Some(syntax_close))
}

fn done_close_span(
    source: &str,
    source_map: &SourceMap<'_>,
    span: Span,
    fallback: Option<Span>,
) -> Option<Span> {
    matching_done_close_span(source, source_map, span).or_else(|| {
        fallback.map(|span| normalized_close_keyword_span(source, source_map, span, "done"))
    })
}

fn normalized_close_keyword_span(
    source: &str,
    source_map: &SourceMap<'_>,
    span: Span,
    keyword: &str,
) -> Span {
    let start = span.start.offset.min(source.len());
    let end = start.saturating_add(keyword.len()).min(source.len());
    if source.get(start..end) == Some(keyword) {
        source_map.span_for_offsets(start, end)
    } else {
        span
    }
}

fn matching_if_close_span(source: &str, source_map: &SourceMap<'_>, span: Span) -> Option<Span> {
    let upper = span.end.offset.min(source.len());
    let mut offset = span.start.offset.min(upper);
    let mut depth = 0usize;
    while offset < upper {
        let ch = source[offset..].chars().next()?;
        match ch {
            '\'' => {
                offset = skip_single_quoted(source, offset + ch.len_utf8(), upper);
                continue;
            }
            '"' => {
                offset = skip_double_quoted(source, offset + ch.len_utf8(), upper);
                continue;
            }
            '#' if shell_comment_can_start(source, offset) => {
                offset = source[offset..]
                    .find('\n')
                    .map_or(upper, |newline| offset + newline + 1);
                continue;
            }
            _ => {}
        }

        if shell_keyword_at(source, offset, upper, "if") {
            depth = depth.saturating_add(1);
            offset += "if".len();
            continue;
        }
        if shell_keyword_at(source, offset, upper, "fi") {
            if depth > 0 {
                depth -= 1;
                if depth == 0 {
                    return Some(source_map.span_for_offsets(offset, offset + 2));
                }
            }
            offset += "fi".len();
            continue;
        }
        offset += ch.len_utf8();
    }
    None
}

fn matching_done_close_span(source: &str, source_map: &SourceMap<'_>, span: Span) -> Option<Span> {
    let upper = span.end.offset.min(source.len());
    let mut offset = span.start.offset.min(upper);
    let mut depth = 0usize;
    while offset < upper {
        let ch = source[offset..].chars().next()?;
        match ch {
            '\'' => {
                offset = skip_single_quoted(source, offset + ch.len_utf8(), upper);
                continue;
            }
            '"' => {
                offset = skip_double_quoted(source, offset + ch.len_utf8(), upper);
                continue;
            }
            '#' if shell_comment_can_start(source, offset) => {
                offset = source[offset..]
                    .find('\n')
                    .map_or(upper, |newline| offset + newline + 1);
                continue;
            }
            _ => {}
        }

        if loop_open_keyword_at(source, offset, upper) {
            depth = depth.saturating_add(1);
            offset += source[offset..]
                .chars()
                .take_while(|ch| ch.is_ascii_alphabetic())
                .map(char::len_utf8)
                .sum::<usize>();
            continue;
        }
        if shell_keyword_at(source, offset, upper, "done") {
            if depth > 0 {
                depth -= 1;
                if depth == 0 {
                    return Some(source_map.span_for_offsets(offset, offset + 4));
                }
            }
            offset += "done".len();
            continue;
        }
        offset += ch.len_utf8();
    }
    None
}

fn loop_open_keyword_at(source: &str, offset: usize, upper: usize) -> bool {
    ["for", "select", "while", "until", "foreach", "repeat"]
        .iter()
        .any(|keyword| shell_keyword_at(source, offset, upper, keyword))
}

fn shell_keyword_at(source: &str, offset: usize, upper: usize, keyword: &str) -> bool {
    let end = offset.saturating_add(keyword.len());
    end <= upper
        && source.get(offset..end) == Some(keyword)
        && shell_keyword_boundaries_match(source, offset, end)
}

fn shell_comment_can_start(source: &str, offset: usize) -> bool {
    source[..offset]
        .chars()
        .next_back()
        .is_none_or(|ch| ch == '\n' || ch.is_whitespace() || matches!(ch, ';' | '&' | '|'))
}

fn skip_single_quoted(source: &str, mut offset: usize, upper: usize) -> usize {
    while offset < upper {
        let Some(ch) = source[offset..].chars().next() else {
            break;
        };
        offset += ch.len_utf8();
        if ch == '\'' {
            break;
        }
    }
    offset
}

fn skip_double_quoted(source: &str, mut offset: usize, upper: usize) -> usize {
    while offset < upper {
        let Some(ch) = source[offset..].chars().next() else {
            break;
        };
        offset += ch.len_utf8();
        if ch == '\\' {
            if let Some(escaped) = source[offset..].chars().next() {
                offset += escaped.len_utf8();
            }
        } else if ch == '"' {
            break;
        }
    }
    offset
}

fn shell_keyword_boundaries_match(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    before.is_none_or(|ch| !is_shell_keyword_char(ch))
        && after.is_none_or(|ch| !is_shell_keyword_char(ch))
}

fn is_shell_keyword_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn gap_has_blank_line(source: &str, start: usize, end: usize) -> bool {
    let lower = start.min(end).min(source.len());
    let upper = start.max(end).min(source.len());
    source
        .get(lower..upper)
        .is_some_and(|gap| gap.bytes().filter(|byte| *byte == b'\n').count() >= 2)
}

fn gap_has_empty_physical_line(source: &str, start: usize, end: usize) -> bool {
    let lower = start.min(end).min(source.len());
    let upper = start.max(end).min(source.len());
    let Some(gap) = source.get(lower..upper) else {
        return false;
    };
    let bytes = gap.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\n' {
            let mut next = index + 1;
            while next < bytes.len() && matches!(bytes[next], b' ' | b'\t' | b'\r') {
                next += 1;
            }
            if next < bytes.len() && bytes[next] == b'\n' {
                return true;
            }
        }
        index += 1;
    }
    false
}

fn gap_starts_with_empty_physical_line(source: &str, start: usize, end: usize) -> bool {
    let lower = start.min(end).min(source.len());
    let upper = start.max(end).min(source.len());
    let Some(gap) = source.get(lower..upper) else {
        return false;
    };
    for byte in gap.bytes() {
        match byte {
            b' ' | b'\t' | b'\r' => {}
            b'\n' => return true,
            _ => return false,
        }
    }
    false
}

fn case_item_body_upper_bound(item: &CaseItem, fallback: usize) -> Option<usize> {
    Some(
        item.terminator_span
            .map(|span| span.start.offset)
            .unwrap_or(fallback),
    )
}

fn case_has_blank_line_after_in(command: &CaseCommand, source: &str) -> bool {
    let Some(first_pattern_start) = command
        .cases
        .first()
        .and_then(|item| item.patterns.first())
        .map(|pattern| pattern.span.start.offset)
    else {
        return false;
    };
    let start = command.word.span.end.offset.min(source.len());
    let end = first_pattern_start.min(source.len());
    let Some(prefix) = source.get(start..end) else {
        return false;
    };
    let Some(in_end) = last_shell_keyword_end(prefix, "in") else {
        return false;
    };
    let gap_start = start + in_end;
    gap_has_empty_physical_line(source, gap_start, end)
}

fn case_command_was_inline_in_source(command: &CaseCommand, source: &str) -> bool {
    command.span.slice(source).lines().nth(1).is_none()
}

fn case_item_has_blank_line_before(previous: &CaseItem, item: &CaseItem, source: &str) -> bool {
    let Some(start) = case_item_source_end_offset(previous, source) else {
        return false;
    };
    let Some(end) = item
        .patterns
        .first()
        .map(|pattern| pattern.span.start.offset)
    else {
        return false;
    };
    gap_has_empty_physical_line(source, start, end)
}

fn case_item_source_end_offset(item: &CaseItem, source: &str) -> Option<usize> {
    let content_end = item
        .body
        .last()
        .map(|stmt| stmt_format_span(stmt).end.offset)
        .or_else(|| item.patterns.last().map(|pattern| pattern.span.end.offset))?;
    if let Some(terminator_span) = item.terminator_span
        && terminator_span.end.offset >= content_end
        && terminator_span.end.offset <= source.len()
    {
        return Some(terminator_span.end.offset);
    }
    let stmt_end = content_end.min(source.len());
    let line_end = source[stmt_end..]
        .find(['\n', '\r'])
        .map_or(source.len(), |offset| stmt_end + offset);
    let terminator = case_terminator(item.terminator);
    let end = source
        .get(stmt_end..line_end)
        .and_then(|tail| {
            tail.find(terminator)
                .map(|offset| stmt_end + offset + terminator.len())
        })
        .unwrap_or(stmt_end);
    Some(end)
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

fn case_item_single_body_stmt_can_inline(stmt: &Stmt) -> bool {
    !matches!(stmt.command, Command::Compound(CompoundCommand::If(_)))
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

fn case_item_pattern_close_paren_on_own_line(item: &CaseItem, source: &str) -> bool {
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
    let line_start = slice[..close_offset]
        .rfind('\n')
        .map_or(0, |offset| offset + 1);
    slice[line_start..close_offset]
        .trim_matches([' ', '\t', '\r'])
        .is_empty()
}

fn case_item_close_paren_shares_line_with_body(item: &CaseItem, source: &str) -> bool {
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
    !slice[close_offset + 1..].contains(['\n', '\r'])
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

fn case_item_has_blank_line_after_pattern(
    item: &CaseItem,
    source: &str,
    first_body_line: usize,
    first_body_stmt_line: usize,
) -> bool {
    let Some(pattern_line) = item.patterns.last().map(|pattern| pattern.span.end.line) else {
        return false;
    };
    let stmt_line = if first_body_line <= pattern_line {
        first_body_stmt_line
    } else {
        first_body_line
    };
    if stmt_line == 0 {
        return false;
    }
    if stmt_line <= pattern_line.saturating_add(1) {
        return false;
    }
    let lines = source.lines().collect::<Vec<_>>();
    ((pattern_line + 1)..stmt_line).any(|line| {
        line.checked_sub(1)
            .and_then(|index| lines.get(index))
            .is_some_and(|text| text.trim_matches([' ', '\t', '\r']).is_empty())
    })
}

fn case_item_has_blank_line_before_terminator(item: &CaseItem, source: &str) -> bool {
    let Some(terminator_start) = item.terminator_span.map(|span| span.start.offset) else {
        return false;
    };
    if item.body.is_empty() {
        return false;
    }
    let content_end = sequence_close_gap_start(&item.body, source);
    gap_has_empty_physical_line(source, content_end, terminator_start)
}

fn case_suffix_comment_region_start(item: &CaseItem, source: &str) -> Option<usize> {
    case_item_source_end_offset(item, source)
}

fn case_suffix_comment_start_line(item: &CaseItem) -> Option<usize> {
    item.terminator_span
        .map(|span| span.end.line)
        .or_else(|| item.body.last().map(|stmt| stmt_span(stmt).end.line))
        .or_else(|| item.patterns.last().map(|pattern| pattern.span.end.line))
}

fn sequence_close_gap_start(commands: &StmtSeq, source: &str) -> usize {
    commands
        .trailing_comments
        .iter()
        .map(|comment| usize::from(comment.range.end()))
        .max()
        .unwrap_or_else(|| branch_body_content_end(commands, source))
}

fn case_has_blank_line_before_esac(command: &CaseCommand, source: &str) -> bool {
    let Some(last_item) = command.cases.last() else {
        return false;
    };
    let Some(start) = case_item_source_end_offset(last_item, source) else {
        return false;
    };
    let end = command.span.end.offset.min(source.len());
    let Some(gap) = source.get(start.min(source.len())..end) else {
        return false;
    };
    let Some(esac_start) = gap.rfind("esac") else {
        return false;
    };
    gap_has_blank_line(source, start, start + esac_start)
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

fn matching_group_close_char(open: char) -> char {
    match open {
        '(' => ')',
        _ => '}',
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
    comment: &SourceComment<'_>,
    current_code_column: usize,
) -> usize {
    let Some(target_column) = trailing_comment_alignment_column(source, comment) else {
        return 1;
    };
    let indent_adjust = trailing_comment_tab_indent_adjust(source, comment);
    target_column
        .saturating_sub(current_code_column.saturating_add(indent_adjust))
        .max(1)
}

fn close_suffix_comment_padding(
    source: &str,
    comment: &SourceComment<'_>,
    current_code_column: usize,
    current_indent_column: usize,
) -> usize {
    if let Some(padding) = aligned_close_suffix_comment_padding(
        source,
        comment,
        current_code_column,
        current_indent_column,
    ) {
        return padding;
    }
    trailing_comment_padding(source, comment, current_code_column)
}

fn aligned_close_suffix_comment_padding(
    source: &str,
    comment: &SourceComment<'_>,
    current_code_column: usize,
    current_indent_column: usize,
) -> Option<usize> {
    let entries = close_suffix_comment_alignment_entries(source, comment)?;
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
    comment: &SourceComment<'_>,
) -> Option<Vec<CloseSuffixAlignmentEntry>> {
    let (line_start, line_end) = line_bounds_for_offset(source, comment.span().start.offset)?;
    let mut entries = vec![close_suffix_alignment_entry(
        source,
        line_start,
        line_end,
        Some(comment.span().start.offset),
    )?];

    let mut previous_start = line_start;
    while let Some((start, end)) = previous_line_bounds(source, previous_start) {
        let Some(entry) = close_suffix_alignment_entry(source, start, end, None) else {
            break;
        };
        entries.push(entry);
        previous_start = start;
    }

    let mut next_start = line_end
        .checked_add(1)
        .filter(|offset| *offset < source.len());
    while let Some(start) = next_start {
        let end = source[start..]
            .find('\n')
            .map_or(source.len(), |offset| start + offset);
        let Some(entry) = close_suffix_alignment_entry(source, start, end, None) else {
            break;
        };
        entries.push(entry);
        next_start = end.checked_add(1).filter(|offset| *offset < source.len());
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

fn trailing_comment_alignment_column(source: &str, comment: &SourceComment<'_>) -> Option<usize> {
    let (line_start, line_end) = line_bounds_for_offset(source, comment.span().start.offset)?;
    let mut widths = vec![trimmed_line_width(
        source.get(line_start..comment.span().start.offset)?,
    )?];

    let mut previous_start = line_start;
    while let Some((start, end)) = previous_line_bounds(source, previous_start) {
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

    let mut next_start = line_end
        .checked_add(1)
        .filter(|offset| *offset < source.len());
    while let Some(start) = next_start {
        let end = source[start..]
            .find('\n')
            .map_or(source.len(), |offset| start + offset);
        let Some(width) = inline_comment_code_width(source, start, end, None) else {
            if let Some((width, suffix_end)) =
                multiline_header_suffix_comment_width(source, line_start, start, end)
            {
                widths.push(width);
                next_start = suffix_end
                    .checked_add(1)
                    .filter(|offset| *offset < source.len());
                continue;
            }
            break;
        };
        widths.push(width);
        next_start = end.checked_add(1).filter(|offset| *offset < source.len());
    }

    (widths.len() > 1).then(|| widths.into_iter().max().unwrap_or(0) + 1)
}

fn trailing_comment_tab_indent_adjust(source: &str, comment: &SourceComment<'_>) -> usize {
    let Some((line_start, line_end)) = line_bounds_for_offset(source, comment.span().start.offset)
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
    while let Some((start, end)) = previous_line_bounds(source, previous_start) {
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

    let mut next_start = line_end
        .checked_add(1)
        .filter(|offset| *offset < source.len());
    while let Some(start) = next_start {
        let end = source[start..]
            .find('\n')
            .map_or(source.len(), |offset| start + offset);
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
            next_start = end.checked_add(1).filter(|offset| *offset < source.len());
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

fn line_bounds_for_offset(source: &str, offset: usize) -> Option<(usize, usize)> {
    if source.is_empty() {
        return None;
    }
    let offset = offset.min(source.len().saturating_sub(1));
    let start = source[..offset]
        .rfind('\n')
        .map_or(0, |index| index.saturating_add(1));
    let end = source[offset..]
        .find('\n')
        .map_or(source.len(), |index| offset + index);
    Some((start, end))
}

fn previous_line_bounds(source: &str, line_start: usize) -> Option<(usize, usize)> {
    if line_start == 0 {
        return None;
    }
    let end = line_start.saturating_sub(1);
    let start = source[..end]
        .rfind('\n')
        .map_or(0, |index| index.saturating_add(1));
    Some((start, end))
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
    current_line_start: usize,
    header_start: usize,
    header_end: usize,
) -> Option<(usize, usize)> {
    let header_line = source.get(header_start..header_end)?;
    if find_inline_comment_start(header_line, header_start).is_some() {
        return None;
    }
    let header = header_line.trim_matches([' ', '\t', '\r']);
    let suffix = multiline_header_suffix_keyword(header)?;

    let suffix_start = header_end.checked_add(1)?;
    if suffix_start >= source.len() {
        return None;
    }
    let suffix_end = source[suffix_start..]
        .find('\n')
        .map_or(source.len(), |offset| suffix_start + offset);
    let suffix_line = source.get(suffix_start..suffix_end)?;
    let comment_offset = find_inline_comment_start(suffix_line, suffix_start)?;
    let suffix_prefix = source.get(suffix_start..comment_offset)?;
    if suffix_prefix.trim_matches([' ', '\t', '\r']) != suffix {
        return None;
    }

    let header = header.trim_end_matches(';').trim_end();
    let rendered = format!("{header}; {suffix}");
    let indent_delta =
        rendered_indent_delta_between_lines(source, current_line_start, header_start);
    Some((
        normalized_comment_alignment_width(&rendered) + indent_delta,
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

fn rendered_indent_delta_between_lines(
    source: &str,
    current_line_start: usize,
    target_line_start: usize,
) -> usize {
    let Some((_, current_line_end)) = line_bounds_for_offset(source, current_line_start) else {
        return 0;
    };
    let Some((_, target_line_end)) = line_bounds_for_offset(source, target_line_start) else {
        return 0;
    };
    let current_indent = line_indent_width(source, current_line_start, current_line_end);
    let target_indent = line_indent_width(source, target_line_start, target_line_end);
    if target_indent <= current_indent {
        return 0;
    }
    let delta = target_indent - current_indent;
    let unit = if current_indent == 0 {
        delta
    } else {
        current_indent.min(delta)
    }
    .max(1);
    delta.div_ceil(unit)
}

fn line_indent_width(source: &str, line_start: usize, line_end: usize) -> usize {
    source
        .get(line_start..line_end)
        .and_then(leading_indent_and_code_start)
        .map_or(0, |(indent, _)| indent)
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
    let redirect_normalized = trim_redirect_padding_for_alignment(&collapsed);
    let array_normalized = trim_compound_assignment_padding_for_alignment(&redirect_normalized);
    let normalized = trim_arithmetic_expansion_padding_for_alignment(&array_normalized);
    normalized.chars().count()
        + case_pattern_pipe_alignment_width(&normalized)
        + moved_function_brace_alignment_width(&normalized)
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
            if let Some(operator_end) = alignment_redirect_operator_end(bytes, operator_start)
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
            && let Some(operator_end) = alignment_redirect_operator_end(bytes, index)
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

fn alignment_redirect_operator_end(bytes: &[u8], start: usize) -> Option<usize> {
    match bytes.get(start).copied()? {
        b'>' => Some(match bytes.get(start + 1).copied() {
            Some(b'>' | b'|' | b'&') => start + 2,
            _ => start + 1,
        }),
        b'<' => Some(match bytes.get(start + 1).copied() {
            Some(b'<' | b'>' | b'&') => {
                if bytes.get(start + 2) == Some(&b'<') {
                    start + 3
                } else {
                    start + 2
                }
            }
            _ => start + 1,
        }),
        _ => None,
    }
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

fn sequence_verbatim_span(statements: &StmtSeq, source_map: &SourceMap<'_>) -> Option<Span> {
    statements
        .iter()
        .map(|stmt| stmt_verbatim_span_with_source_map(stmt, source_map))
        .reduce(|left, right| left.merge(right))
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
    comment: &SourceComment<'_>,
    pattern_start: usize,
    disabled_case_pattern_context: bool,
) -> bool {
    let Some(comment_indent) = line_indent_before_offset(source, comment.span().start.offset)
    else {
        return false;
    };
    let Some(pattern_indent) = line_indent_before_offset(source, pattern_start) else {
        return false;
    };
    let comment_width = shell_indent_width(comment_indent);
    let pattern_width = shell_indent_width(pattern_indent);
    if disabled_case_pattern_context && comment_looks_like_disabled_case_pattern(comment) {
        return comment_width < pattern_width;
    }
    if comment_width < pattern_width && case_prefix_comment_follows_terminator(source, comment) {
        return true;
    }
    comment_width > pattern_width || (comment_width == 0 && pattern_width > 0)
}

fn case_prefix_comment_follows_terminator(source: &str, comment: &SourceComment<'_>) -> bool {
    let Some((line_start, _)) = line_bounds_for_offset(source, comment.span().start.offset) else {
        return false;
    };
    let Some((previous_start, previous_end)) = previous_line_bounds(source, line_start) else {
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

fn shell_indent_width(indent: &str) -> usize {
    indent.chars().count()
}

fn comment_precedes_close_keyword_at_same_indent(
    source: &str,
    comment: &SourceComment<'_>,
) -> bool {
    let Some(comment_indent) = line_indent_before_offset(source, comment.span().start.offset)
    else {
        return false;
    };
    let mut offset = source
        .get(comment.span().end.offset..)
        .and_then(|suffix| {
            suffix
                .find('\n')
                .map(|line_end| comment.span().end.offset + line_end + 1)
        })
        .unwrap_or(source.len());

    while offset < source.len() {
        let line_end = source[offset..]
            .find('\n')
            .map_or(source.len(), |line_end| offset + line_end);
        let Some(line) = source.get(offset..line_end) else {
            return false;
        };
        let trimmed = line.trim_start_matches([' ', '\t']);
        if trimmed.trim().is_empty() {
            offset = line_end.saturating_add(1);
            continue;
        }
        let indent_len = line.len() - trimmed.len();
        if line.get(..indent_len) == Some(comment_indent) {
            if starts_with_outdent_preserving_close_keyword(trimmed) {
                return true;
            }
            if trimmed.starts_with('#') {
                offset = line_end.saturating_add(1);
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

fn collect_pipeline<'a>(
    command: &'a BinaryCommand,
    statements: &mut Vec<&'a Stmt>,
    operators: &mut Vec<(BinaryOp, Span)>,
) {
    collect_pipeline_stmt(&command.left, statements, operators);
    operators.push((command.op, command.op_span));
    collect_pipeline_stmt(&command.right, statements, operators);
}

fn collect_pipeline_stmt<'a>(
    stmt: &'a Stmt,
    statements: &mut Vec<&'a Stmt>,
    operators: &mut Vec<(BinaryOp, Span)>,
) {
    if let Command::Binary(binary) = &stmt.command
        && stmt.redirects.is_empty()
        && !stmt.negated
        && stmt.terminator.is_none()
        && matches!(binary.op, BinaryOp::Pipe | BinaryOp::PipeAll)
    {
        collect_pipeline(binary, statements, operators);
    } else {
        statements.push(stmt);
    }
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

fn line_has_trailing_continuation_backslash(line: &str) -> bool {
    line.trim_end_matches([' ', '\t', '\r', '\n'])
        .ends_with('\\')
}

fn pipeline_operator_breaks(
    statements: &[&Stmt],
    operators: &[(BinaryOp, Span)],
    source: &str,
    source_map: &SourceMap<'_>,
    options: &ResolvedShellFormatOptions,
) -> Vec<bool> {
    let mut breaks = Vec::with_capacity(operators.len());
    for index in 1..statements.len() {
        let Some((_, operator_span)) = operators.get(index - 1) else {
            continue;
        };
        let previous_end = stmt_attachment_span(statements[index - 1], source, source_map, options)
            .end
            .offset;
        let next_start = stmt_attachment_span(statements[index], source, source_map, options)
            .start
            .offset;
        breaks.push(
            pipeline_operator_starts_or_ends_line(source, *operator_span)
                || has_newline_between_offsets(source, previous_end, operator_span.start.offset)
                || has_newline_between_offsets(source, operator_span.end.offset, next_start),
        );
    }

    breaks
}

fn pipeline_operator_starts_or_ends_line(source: &str, operator_span: Span) -> bool {
    let start = operator_span.start.offset;
    let end = operator_span.end.offset;
    if start >= end || end > source.len() {
        return false;
    }

    let line_start = source[..start]
        .rfind('\n')
        .map_or(0, |offset| offset.saturating_add(1));
    let line_end = source[end..]
        .find('\n')
        .map_or(source.len(), |offset| end.saturating_add(offset));
    let has_previous_line = line_start > 0;
    let has_next_line = line_end < source.len();
    let before = &source[line_start..start];
    let after = &source[end..line_end];

    (has_previous_line && line_edge_is_blank_or_continuation(before))
        || (has_next_line && line_edge_is_blank_or_continuation(after))
}

fn line_edge_is_blank_or_continuation(text: &str) -> bool {
    let trimmed = text.trim_matches(|ch| matches!(ch, ' ' | '\t' | '\r'));
    trimmed.is_empty() || trimmed == "\\"
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

fn pipeline_interstitial_comment_end(stmt: &Stmt, source_map: &SourceMap<'_>) -> usize {
    let group_span = match &stmt.command {
        Command::Compound(CompoundCommand::BraceGroup(commands)) => {
            group_attachment_span(commands.as_slice(), source_map, '{', '}')
        }
        Command::Compound(CompoundCommand::Subshell(commands)) => {
            group_attachment_span(commands.as_slice(), source_map, '(', ')')
        }
        _ => None,
    };
    group_span
        .map(|span| span.start.offset)
        .unwrap_or_else(|| command_format_span(&stmt.command).start.offset)
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
    match options.indent_style() {
        IndentStyle::Tab => {
            for _ in 0..column {
                rendered.push('\t');
            }
        }
        IndentStyle::Space => {
            for _ in 0..column {
                rendered.push(' ');
            }
        }
    }
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

#[derive(Clone, Copy)]
enum CommandSubstitutionPipelineContinuation {
    None,
    Comment,
    StructuralPipe { line_started_in_quote: bool },
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

#[derive(Default)]
struct RenderedLineQuoteState {
    single_quoted: bool,
    double_quoted: bool,
    escaped: bool,
}

impl RenderedLineQuoteState {
    fn in_quote(&self) -> bool {
        self.single_quoted || self.double_quoted
    }
}

fn command_substitution_pipeline_stage_continuation(
    line: &str,
    was_pipeline_stage: bool,
    quote_state: &mut RenderedLineQuoteState,
) -> CommandSubstitutionPipelineContinuation {
    let content = line.trim_end_matches(['\r', '\n']);
    let scan_start = command_substitution_context_start(content).unwrap_or(0);
    if scan_start > 0 {
        *quote_state = RenderedLineQuoteState::default();
    }
    let content = &content[scan_start..];

    if was_pipeline_stage
        && !quote_state.in_quote()
        && content.trim_start_matches([' ', '\t']).starts_with('#')
    {
        return CommandSubstitutionPipelineContinuation::Comment;
    }
    let line_started_in_quote = quote_state.in_quote();
    if rendered_line_ends_with_structural_pipe_continuation_in_quote_state(content, quote_state) {
        CommandSubstitutionPipelineContinuation::StructuralPipe {
            line_started_in_quote,
        }
    } else {
        CommandSubstitutionPipelineContinuation::None
    }
}

fn rendered_line_ends_with_structural_pipe_continuation_in_quote_state(
    line: &str,
    quote_state: &mut RenderedLineQuoteState,
) -> bool {
    let trimmed = line.trim_end_matches([' ', '\t', '\r']);
    let pipe_offset = final_pipe_operator_bounds(trimmed).map(|(offset, _)| offset);
    let mut pipe_is_unquoted = false;

    for (offset, ch) in trimmed.char_indices() {
        if pipe_offset == Some(offset) {
            pipe_is_unquoted = !quote_state.in_quote() && !quote_state.escaped;
            break;
        }

        if quote_state.escaped {
            quote_state.escaped = false;
            continue;
        }

        match ch {
            '\\' if !quote_state.single_quoted => quote_state.escaped = true,
            '\'' if !quote_state.double_quoted => {
                quote_state.single_quoted = !quote_state.single_quoted;
            }
            '"' if !quote_state.single_quoted => {
                quote_state.double_quoted = !quote_state.double_quoted;
            }
            _ => {}
        }
    }

    quote_state.escaped = false;
    pipe_is_unquoted
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
        let (line, next, had_newline) = match rest.find('\n') {
            Some(index) => (&rest[..index], &rest[index + 1..], true),
            None => (rest, "", false),
        };

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

fn rendered_line_opens_command_substitution_pipeline(line: &str) -> bool {
    if !rendered_line_ends_with_structural_pipe_continuation(line) {
        return false;
    }

    command_substitution_context_start(line).is_some()
        && line
            .bytes()
            .take_while(|byte| matches!(*byte, b' ' | b'\t'))
            .any(|byte| byte == b' ')
}

fn rendered_line_ends_with_structural_pipe_continuation(line: &str) -> bool {
    let trimmed = line.trim_end_matches([' ', '\t', '\r']);
    let Some((pipe_offset, scan_end)) = final_pipe_operator_bounds(trimmed) else {
        return false;
    };
    let scan_start = command_substitution_context_start(&trimmed[..pipe_offset]).unwrap_or(0);

    final_pipe_operator_is_unquoted(&trimmed[scan_start..scan_end])
}

fn final_pipe_operator_bounds(line: &str) -> Option<(usize, usize)> {
    if line.ends_with("|&") {
        Some((line.len().saturating_sub(2), line.len()))
    } else if line.ends_with('|') && !line.ends_with("||") {
        Some((line.len().saturating_sub(1), line.len()))
    } else {
        None
    }
}

fn command_substitution_context_start(line: &str) -> Option<usize> {
    line.rfind("$(")
        .or_else(|| line.rfind("<("))
        .or_else(|| line.rfind(">("))
        .map(|offset| offset.saturating_add(2))
}

fn final_pipe_operator_is_unquoted(text: &str) -> bool {
    let Some((pipe_offset, _)) =
        final_pipe_operator_bounds(text.trim_end_matches([' ', '\t', '\r']))
    else {
        return false;
    };
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;

    for (offset, ch) in text.char_indices() {
        if offset == pipe_offset {
            return !single_quoted && !double_quoted && !escaped;
        }

        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' if !single_quoted => escaped = true,
            '\'' if !double_quoted => single_quoted = !single_quoted,
            '"' if !single_quoted => double_quoted = !double_quoted,
            _ => {}
        }
    }

    false
}

fn has_newline_between_offsets(source: &str, start: usize, end: usize) -> bool {
    let lower = start.min(end).min(source.len());
    let upper = start.max(end).min(source.len());
    source
        .get(lower..upper)
        .is_some_and(|between| between.contains('\n'))
}

fn conditional_binary_has_explicit_rhs_break(
    expression: &ConditionalBinaryExpr,
    source: &str,
) -> bool {
    pipeline_operator_starts_or_ends_line(source, expression.op_span)
        || has_newline_between_offsets(
            source,
            expression.left.span().end.offset,
            expression.op_span.start.offset,
        )
        || has_newline_between_offsets(
            source,
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

fn word_contains_command_substitution(word: &Word) -> bool {
    word.parts
        .iter()
        .any(|part| word_part_contains_command_substitution(&part.kind))
}

fn word_contains_process_substitution(word: &Word) -> bool {
    word.parts
        .iter()
        .any(|part| word_part_contains_process_substitution(&part.kind))
}

fn word_part_contains_command_substitution(part: &WordPart) -> bool {
    match part {
        WordPart::CommandSubstitution { .. } => true,
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter()
            .any(|part| word_part_contains_command_substitution(&part.kind)),
        WordPart::ArithmeticExpansion {
            expression_word_ast,
            ..
        } => word_contains_command_substitution(expression_word_ast),
        WordPart::Parameter(_) => false,
        WordPart::ParameterExpansion {
            operand_word_ast, ..
        } => operand_word_ast
            .as_deref()
            .is_some_and(word_contains_command_substitution),
        WordPart::IndirectExpansion {
            operand_word_ast, ..
        } => operand_word_ast
            .as_deref()
            .is_some_and(word_contains_command_substitution),
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
            word_contains_command_substitution(offset_word_ast)
                || length_word_ast
                    .as_deref()
                    .is_some_and(word_contains_command_substitution)
        }
        _ => false,
    }
}

fn word_part_contains_process_substitution(part: &WordPart) -> bool {
    match part {
        WordPart::ProcessSubstitution { .. } => true,
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter()
            .any(|part| word_part_contains_process_substitution(&part.kind)),
        WordPart::ArithmeticExpansion {
            expression_word_ast,
            ..
        } => word_contains_process_substitution(expression_word_ast),
        WordPart::ParameterExpansion {
            operand_word_ast, ..
        }
        | WordPart::IndirectExpansion {
            operand_word_ast, ..
        } => operand_word_ast
            .as_deref()
            .is_some_and(word_contains_process_substitution),
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
            word_contains_process_substitution(offset_word_ast)
                || length_word_ast
                    .as_deref()
                    .is_some_and(word_contains_process_substitution)
        }
        _ => false,
    }
}

fn collect_command_list_first<'a>(
    command: &'a BinaryCommand,
    rest: &mut Vec<BinaryListItem<'a>>,
) -> &'a Stmt {
    if let Command::Binary(left_binary) = &command.left.command
        && command.left.redirects.is_empty()
        && !command.left.negated
        && command.left.terminator.is_none()
        && matches!(left_binary.op, BinaryOp::And | BinaryOp::Or)
    {
        let first = collect_command_list_first(left_binary, rest);
        rest.push(BinaryListItem {
            operator: command.op,
            operator_span: command.op_span,
            stmt: &command.right,
        });
        return first;
    }

    let first = command.left.as_ref();
    rest.push(BinaryListItem {
        operator: command.op,
        operator_span: command.op_span,
        stmt: &command.right,
    });
    first
}

fn list_item_inline_separator(operator: BinaryOp) -> &'static str {
    match operator {
        BinaryOp::And => " && ",
        BinaryOp::Or => " || ",
        BinaryOp::Pipe | BinaryOp::PipeAll => "; ",
    }
}

fn list_item_multiline_separator(operator: BinaryOp) -> &'static str {
    match operator {
        BinaryOp::And => " &&",
        BinaryOp::Or => " ||",
        BinaryOp::Pipe | BinaryOp::PipeAll => ";",
    }
}

fn if_branch_upper_bound(
    command: &IfCommand,
    branch_index: usize,
    source: &str,
    source_map: &SourceMap<'_>,
) -> usize {
    if let Some((start, end)) = if_next_branch_region(command, branch_index, source) {
        branch_prefix_comments(source, start, end)
            .first()
            .map(|comment| comment.offset)
            .unwrap_or(end)
    } else {
        if_close_span(source, source_map, command)
            .map(|span| span.start.offset)
            .unwrap_or(command.span.end.offset)
    }
}

fn if_next_branch_has_blank_line_before_keyword(
    command: &IfCommand,
    branch_index: usize,
    source: &str,
) -> bool {
    if_next_branch_region(command, branch_index, source).is_some_and(|(start, end)| {
        let first_prefix = branch_prefix_comments(source, start, end)
            .first()
            .map(|comment| comment.offset)
            .unwrap_or(end);
        gap_has_empty_physical_line(source, start, first_prefix)
    })
}

fn if_branch_prefix_comments_have_blank_line_before_keyword(
    command: &IfCommand,
    branch_index: usize,
    source: &str,
) -> bool {
    if_next_branch_region(command, branch_index, source).is_some_and(|(start, end)| {
        let comments = branch_prefix_comments(source, start, end);
        let Some(last) = comments.last() else {
            return false;
        };
        let Some(line_end) = source[last.offset..end].find('\n') else {
            return false;
        };
        gap_has_empty_physical_line(source, last.offset + line_end, end)
    })
}

fn if_next_branch_region(
    command: &IfCommand,
    branch_index: usize,
    source: &str,
) -> Option<(usize, usize)> {
    let current_branch_end = if branch_index == 0 {
        branch_body_content_end(&command.then_branch, source)
    } else {
        command
            .elif_branches
            .get(branch_index - 1)
            .map(|(_, body)| branch_body_content_end(body, source))
            .unwrap_or_else(|| branch_body_content_end(&command.then_branch, source))
    };

    if let Some((condition, _)) = command.elif_branches.get(branch_index) {
        let keyword = branch_keyword_offset(
            source,
            current_branch_end,
            condition.span.start.offset,
            "elif",
        )
        .unwrap_or(condition.span.start.offset);
        Some((current_branch_end, keyword))
    } else if branch_index == command.elif_branches.len() {
        command.else_branch.as_ref().map(|body| {
            let keyword =
                branch_keyword_offset(source, current_branch_end, body.span.start.offset, "else")
                    .unwrap_or(body.span.start.offset);
            (current_branch_end, keyword)
        })
    } else {
        None
    }
}

fn branch_body_content_end(body: &StmtSeq, source: &str) -> usize {
    let mut end = body
        .last()
        .map(|stmt| stmt_span(stmt).end.offset)
        .unwrap_or(body.span.end.offset);
    if let Some(stmt) = body.last() {
        for redirect in &stmt.redirects {
            let Some(heredoc) = redirect.heredoc() else {
                continue;
            };
            let heredoc_end = heredoc_closing_marker_bounds(heredoc, source)
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

fn branch_keyword_offset(source: &str, start: usize, end: usize, keyword: &str) -> Option<usize> {
    let start = start.min(end).min(source.len());
    let end = end.min(source.len());
    let mut line_start = start;
    while line_start < end {
        let line_end = source[line_start..end]
            .find('\n')
            .map_or(end, |offset| line_start + offset);
        let line = source.get(line_start..line_end)?;
        let mut search_start = 0;
        while let Some(relative) = line[search_start..].find(keyword) {
            let keyword_start = search_start + relative;
            let keyword_end = keyword_start + keyword.len();
            if branch_keyword_candidate_matches(line, keyword_start, keyword_end) {
                return Some(line_start + keyword_start);
            }
            search_start = keyword_end;
        }
        line_start = line_end.saturating_add(1);
    }
    None
}

fn branch_keyword_candidate_matches(line: &str, start: usize, end: usize) -> bool {
    if !shell_keyword_boundaries_match(line, start, end) {
        return false;
    }

    let prefix = &line[..start];
    let trimmed = prefix.trim_start_matches([' ', '\t']);
    if trimmed.starts_with('#') {
        return false;
    }

    let before = prefix.trim_end_matches([' ', '\t']);
    before.is_empty() || before.ends_with(';') || before.ends_with('&')
}

#[derive(Debug, Clone)]
struct BranchPrefixComment {
    offset: usize,
    text: String,
    source_indent: usize,
}

fn branch_prefix_comments(source: &str, start: usize, end: usize) -> Vec<BranchPrefixComment> {
    let start = start.min(end).min(source.len());
    let end = end.min(source.len());
    let Some(slice) = source.get(start..end) else {
        return Vec::new();
    };
    let keyword_indent = line_indent_before_offset(source, end).unwrap_or("");

    let mut comments = Vec::new();
    let mut in_branch_prefix_run = false;
    let mut offset = start;
    for line in slice.split_inclusive('\n') {
        let text = line.trim_end_matches(['\n', '\r']);
        let trimmed = text.trim_start_matches([' ', '\t']);
        let indent = text.len().saturating_sub(trimmed.len());
        if trimmed.starts_with('#')
            && (in_branch_prefix_run || text.get(..indent) == Some(keyword_indent))
        {
            comments.push(BranchPrefixComment {
                offset: offset + indent,
                text: trimmed.trim_end_matches([' ', '\t', '\r']).to_string(),
                source_indent: indent,
            });
            in_branch_prefix_run = true;
        } else if !trimmed.is_empty() {
            in_branch_prefix_run = false;
        }
        offset += line.len();
    }
    comments
}

fn comment_looks_like_disabled_if_branch(text: &str) -> bool {
    let body = text
        .strip_prefix('#')
        .unwrap_or(text)
        .trim_start_matches([' ', '\t']);
    ["elif", "else"]
        .iter()
        .any(|keyword| shell_keyword_prefix_matches(body, keyword))
}

fn branch_prefix_comments_use_disabled_body_indent(comments: &[(usize, String, usize)]) -> bool {
    let Some((_, first_text, first_indent)) = comments.first() else {
        return false;
    };
    comment_looks_like_disabled_if_branch(first_text)
        && comments
            .iter()
            .skip(1)
            .any(|(_, _, indent)| indent > first_indent)
}

fn shell_keyword_prefix_matches(text: &str, keyword: &str) -> bool {
    text.starts_with(keyword) && shell_keyword_boundaries_match(text, 0, keyword.len())
}

fn time_inner_stmt_needs_trailing_comment(stmt: &Stmt) -> bool {
    match &stmt.command {
        Command::Simple(_) | Command::Builtin(_) | Command::Decl(_) => true,
        Command::Compound(CompoundCommand::Arithmetic(_) | CompoundCommand::Conditional(_)) => true,
        Command::Binary(command) => time_inner_stmt_needs_trailing_comment(&command.right),
        _ => false,
    }
}

fn own_line_comments_in_region(source: &str, start: usize, end: usize) -> Vec<BranchPrefixComment> {
    let start = start.min(end).min(source.len());
    let end = end.min(source.len());
    let Some(next_line_start) = source
        .get(start..end)
        .and_then(|slice| slice.find('\n').map(|offset| start + offset + 1))
    else {
        return Vec::new();
    };
    let Some(slice) = source.get(next_line_start..end) else {
        return Vec::new();
    };

    let mut comments = Vec::new();
    let mut offset = next_line_start;
    for line in slice.split_inclusive('\n') {
        let text = line.trim_end_matches(['\n', '\r']);
        let trimmed = text.trim_start_matches([' ', '\t']);
        let indent = text.len().saturating_sub(trimmed.len());
        if trimmed.starts_with('#') {
            comments.push(BranchPrefixComment {
                offset: offset + indent,
                text: trimmed.trim_end_matches([' ', '\t', '\r']).to_string(),
                source_indent: indent,
            });
        }
        offset += line.len();
    }
    comments
}

fn line_indent_before_offset(source: &str, offset: usize) -> Option<&str> {
    let offset = offset.min(source.len());
    let bytes = source.as_bytes();
    let mut line_start = offset;
    while line_start > 0 && bytes.get(line_start - 1) != Some(&b'\n') {
        line_start -= 1;
    }
    let line = source.get(line_start..offset)?;
    let indent_end = line
        .char_indices()
        .find(|(_, ch)| !matches!(ch, ' ' | '\t'))
        .map_or(line.len(), |(index, _)| index);
    line.get(..indent_end)
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
    let trimmed = between.trim_start_matches(|ch| matches!(ch, ' ' | '\t' | '\r' | '\n'));
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
    let trimmed = text.trim_end_matches(|ch| matches!(ch, ' ' | '\t' | '\r' | '\n'));
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
