use std::fmt::Write as _;
use std::mem;

use shuck_ast::{
    AlwaysCommand, AnonymousFunctionCommand, ArithmeticCommand, ArithmeticForCommand, ArrayElem,
    Assignment, AssignmentValue, BinaryCommand, BinaryOp, BuiltinCommand, CaseCommand, CaseItem,
    Command, CompoundCommand, ConditionalBinaryExpr, ConditionalCommand, ConditionalExpr,
    ConditionalParenExpr, ConditionalUnaryExpr, ConditionalUnaryOp, CoprocCommand, DeclClause,
    DeclOperand, File, ForCommand, ForSyntax, ForeachCommand, ForeachSyntax, FunctionDef, Heredoc,
    IfCommand, IfSyntax, Pattern, Redirect, RedirectKind, RepeatCommand, RepeatSyntax,
    SelectCommand, SimpleCommand, Span, Stmt, StmtSeq, StmtTerminator, TimeCommand, UntilCommand,
    VarRef, WhileCommand, Word,
};
use shuck_format::{IndentStyle, LineEnding};

use crate::Result;
use crate::command::{
    binary_operator, case_terminator, command_format_span, format_arithmetic_command_source,
    format_arithmetic_for_init_source, group_attachment_span, line_gap_break_count,
    multiline_compound_assignment_layout, multiline_compound_assignment_lines,
    render_assignment_head_to_buf, render_assignment_with_facts_to_buf, render_background_operator,
    render_var_ref_to_buf, slice_span, stmt_format_span, stmt_render_start_line, stmt_span,
    stmt_verbatim_span,
};
use crate::comments::{SourceComment, SourceMap};
use crate::facts::FormatterFacts;
use crate::options::ResolvedShellFormatOptions;
use crate::word::{
    render_heredoc_body_to_buf, render_pattern_syntax_to_buf, render_word_syntax_with_facts_to_buf,
    word_has_multiline_literal_source,
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
            } else {
                self.write_text(line);
                active_heredoc = rendered_heredoc_tail_start(line);
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
            if heredoc.strip_tabs && !heredoc.delimiter.starts_with('\t') {
                self.write_indent();
            }
            self.write_verbatim(&heredoc.delimiter);
        }
    }

    fn write_indented_heredoc_text(&mut self, text: &str) {
        let prefix = self.indent_prefix_for_level(self.indent_level.saturating_add(1));
        let mut rest = text;
        while !rest.is_empty() {
            let (line, next) = match rest.find('\n') {
                Some(index) => rest.split_at(index + 1),
                None => (rest, ""),
            };
            let content = line.trim_end_matches(['\r', '\n']);
            if !content.is_empty() && !line.starts_with('\t') {
                self.push_output_str(&prefix);
            }
            self.push_output_str(line);
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
        if word_has_multiline_literal_source(word, self.source()) {
            self.write_rendered_shell_text(&scratch);
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

    fn write_var_ref(&mut self, reference: &VarRef) {
        self.write_rendered(|scratch, source, _| {
            render_var_ref_to_buf(reference, source, scratch);
        });
    }

    fn write_assignment(&mut self, assignment: &Assignment) {
        if assignment_has_raw_backslash_continuation_literal(assignment, self.source()) {
            self.write_rendered_shell_text(assignment.span.slice(self.source()));
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
        if assignment_has_multiline_literal_source(assignment, self.source()) {
            self.write_rendered_shell_text(&scratch);
        } else if rendered_shell_text_has_heredoc_tail(&scratch) {
            self.write_rendered_shell_text_preserving_heredoc_tails(&scratch);
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
            self.write_comment(comment);
            if let Some(next) = comments.get(index + 1) {
                self.write_line_breaks(line_gap_break_count(comment.line(), next.line()));
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
            && let Some(span) = sequence_verbatim_span(statements, source)
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
                    if self.facts().background_has_explicit_line_break(stmt) {
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

        let mut previous_assignment = None;
        let mut previous_end = None;
        for assignment in &command.assignments {
            if previous_assignment.is_some_and(|assignment| {
                assignment_source_has_command_substitution(assignment, self.source())
            }) && previous_end.is_some_and(|previous_end| {
                has_newline_between_offsets(
                    self.source(),
                    previous_end,
                    assignment.span.start.offset,
                )
            }) {
                self.line_continuation();
            } else {
                self.write_command_gap(previous_end, assignment.span.start.offset);
            }
            self.write_assignment(assignment);
            previous_assignment = Some(assignment);
            previous_end = Some(assignment.span.end.offset);
        }
        previous_end =
            self.write_rendered_name_if_nonempty(&rendered_name, previous_end, command.name.span);
        self.restore_scratch_buffer(rendered_name);
        for argument in &command.args {
            self.write_command_gap(previous_end, argument.span.start.offset);
            self.write_word(argument);
            previous_end = Some(argument.span.end.offset);
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

        let mut previous_part = None;
        let mut previous_end = None;
        for part in parts {
            if matches!(
                (previous_part, part),
                (
                    Some(SimpleCommandPart::Assignment(previous_assignment)),
                    SimpleCommandPart::Assignment(_)
                ) if assignment_source_has_command_substitution(previous_assignment, self.source())
            ) && previous_end.is_some_and(|previous_end| {
                has_newline_between_offsets(self.source(), previous_end, part.start_offset(command))
            }) {
                self.line_continuation();
            } else if let SimpleCommandPart::Redirect(redirect) = &part {
                self.write_redirect_gap(previous_end, redirect);
            } else {
                self.write_command_gap(previous_end, part.start_offset(command));
            }
            let end_offset = part.end_offset(command);
            match part {
                SimpleCommandPart::Assignment(assignment) => self.write_assignment(assignment),
                SimpleCommandPart::Name => self.write_text(&rendered_name),
                SimpleCommandPart::Argument(argument) => self.write_word(argument),
                SimpleCommandPart::Redirect(redirect) => self.format_redirect(redirect),
            }
            previous_part = Some(part);
            previous_end = Some(end_offset);
        }
        self.restore_scratch_buffer(rendered_name);
    }

    fn write_redirect_gap(&mut self, previous_end: Option<usize>, redirect: &Redirect) {
        let Some(previous_end) = previous_end else {
            return;
        };
        if previous_end == redirect.span.start.offset {
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
            previous_end = Some(word.span.end.offset);
        }
    }

    fn format_do_done_body(
        &mut self,
        body: &StmtSeq,
        enclosing_span: Span,
        close: &'static str,
    ) -> Result<()> {
        if self.can_inline_body(body, enclosing_span) {
            self.write_text("; do ");
            self.format_inline_stmts(body)?;
            self.write_text("; ");
            self.write_text(close);
            return Ok(());
        }

        if self.body_starts_with_inline_do_brace_group(body) {
            self.write_text("; do ");
            self.format_stmt(&body[0])?;
            self.write_text(self.inline_do_brace_group_done_separator(body, enclosing_span));
            self.write_text(close);
            return Ok(());
        }

        self.write_text("; do");
        let preserve_open_blank = body_has_blank_line_after_keyword(
            self.source(),
            self.source_map(),
            enclosing_span.start.offset,
            "do",
            body,
        );
        self.format_body_with_upper_bound_and_open_blank(
            body,
            Some(enclosing_span.end.offset),
            preserve_open_blank,
        )?;
        if source_has_blank_line_before_last_keyword(self.source(), enclosing_span, close) {
            self.newline();
        }
        self.finish_block(close);
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

        for line in &layout.lines[body_start..] {
            self.newline();
            self.write_indent_units(1);
            self.write_text(line);
        }
        if layout.close_inline {
            self.write_text(")");
        } else {
            self.newline();
            self.write_text(")");
        }
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
            self.with_indent(|formatter| {
                for (index, line) in layout.lines[body_start..].iter().enumerate() {
                    if index > 0 {
                        formatter.newline();
                    }
                    formatter.write_text(line);
                }
            });
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

        let mut operator_breaks = pipeline_operator_breaks(&statements, &operators, self.source());
        if self.facts().pipeline_has_explicit_line_break(pipeline)
            && !operator_breaks.iter().any(|broken| *broken)
        {
            operator_breaks.fill(true);
        }
        let operator_next_line = self.options().binary_next_line();

        for (index, stmt) in statements.iter().enumerate() {
            if index > 0 {
                let operator = operators
                    .get(index - 1)
                    .map(|(operator, _)| binary_operator(operator))
                    .unwrap_or("|");
                let break_here = operator_breaks.get(index - 1).copied().unwrap_or(false);
                if break_here && operator_next_line {
                    self.line_continuation();
                    self.with_indent(|formatter| {
                        formatter.write_text(operator);
                        formatter.write_space();
                        formatter.format_stmt(stmt)
                    })?;
                    continue;
                }
                if break_here {
                    self.write_space();
                    self.write_text(operator);
                    self.newline();
                    self.with_indent(|formatter| formatter.format_stmt(stmt))?;
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
            self.with_indent(|formatter| formatter.format_stmt(item.stmt))?;
            return Ok(());
        }

        self.write_text(list_item_inline_separator(item.operator));
        self.format_stmt(item.stmt)
    }

    fn format_if(&mut self, command: &IfCommand) -> Result<()> {
        match command.syntax {
            IfSyntax::ThenFi { .. } => self.format_then_fi_if(command),
            IfSyntax::Brace { .. } => self.format_brace_if(command),
        }
    }

    fn format_then_fi_if(&mut self, command: &IfCommand) -> Result<()> {
        let source = self.source();
        let then_span = match command.syntax {
            IfSyntax::ThenFi { then_span, .. } => then_span,
            IfSyntax::Brace { .. } => unreachable!("brace if cannot be formatted as then/fi"),
        };

        if command.elif_branches.is_empty()
            && let Some(raw_condition) = raw_grouped_if_condition(command, then_span, source)
        {
            self.write_text("if");
            self.write_text(&raw_condition);
            self.write_text("then");
            let then_upper_bound = if_branch_upper_bound(command, 0, source);
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
                let body_upper_bound = command.span.end.offset;
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
            return Ok(());
        }

        if command.elif_branches.is_empty() && if_condition_starts_after_keyword(command) {
            self.write_text("if");
            self.newline();
            self.with_indent(|formatter| {
                formatter.format_stmt_sequence(&command.condition, Some(then_span.start.offset))
            })?;
            self.newline();
            self.write_text("then");
            let then_upper_bound = if_branch_upper_bound(command, 0, source);
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
                let body_upper_bound = command.span.end.offset;
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
            return Ok(());
        }

        self.write_text("if ");
        self.format_inline_stmts(&command.condition)?;
        let then_separator = self.then_separator_for_condition(&command.condition);
        if command.elif_branches.is_empty()
            && command.else_branch.is_none()
            && self.can_inline_body(&command.then_branch, command.span)
        {
            self.write_text(then_separator);
            self.write_space();
            self.format_inline_stmts(&command.then_branch)?;
            self.write_text("; fi");
            return Ok(());
        }
        if command.elif_branches.is_empty()
            && let Some(else_branch) = &command.else_branch
            && self.can_inline_body(&command.then_branch, command.span)
            && self.can_inline_body(else_branch, command.span)
        {
            self.write_text(then_separator);
            self.write_space();
            self.format_inline_stmts(&command.then_branch)?;
            self.write_text("; else ");
            self.format_inline_stmts(else_branch)?;
            self.write_text("; fi");
            return Ok(());
        }
        if command.elif_branches.is_empty()
            && let Some(else_branch) = &command.else_branch
            && self.can_inline_body(&command.then_branch, command.span)
            && !self.can_inline_body(else_branch, command.span)
            && !self.options().compact_layout()
        {
            self.write_text(then_separator);
            self.write_space();
            self.format_inline_stmts(&command.then_branch)?;
            self.write_text("; else");
            let body_upper_bound = command.span.end.offset;
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
            let then_upper_bound = if_branch_upper_bound(command, 0, source);
            if self.if_final_branch_has_blank_line_before_fi(command, then_upper_bound) {
                self.newline();
            }
            self.newline();
            self.write_text("fi");
            return Ok(());
        }

        self.write_text(then_separator);
        let then_upper_bound = if_branch_upper_bound(command, 0, source);
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
                self.emit_branch_prefix_comments(command, index);
                self.newline();
                self.write_text("elif ");
                self.format_inline_stmts(condition)?;
                self.write_text(self.then_separator_for_condition(condition));
            }
            let body_upper_bound = if_branch_upper_bound(command, index + 1, source);
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
                self.emit_branch_prefix_comments(command, command.elif_branches.len());
                self.newline();
                self.write_text("else");
            }
            let body_upper_bound = command.span.end.offset;
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
        } else {
            if self.if_final_branch_has_blank_line_before_fi(command, then_upper_bound) {
                self.newline();
            }
            self.newline();
            self.write_text("fi");
        }
        Ok(())
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
                )
            })
            .collect::<Vec<_>>();
        if comments.is_empty() {
            return;
        }
        self.newline();
        for (index, (line, text)) in comments.iter().enumerate() {
            self.write_text(text);
            if let Some((next_line, _)) = comments.get(index + 1) {
                self.write_line_breaks(line_gap_break_count(*line, *next_line));
            }
        }
    }

    fn write_sequence_open_suffix(&mut self, commands: &StmtSeq, upper_bound: Option<usize>) {
        let Some(span) = self
            .facts()
            .sequence(commands, upper_bound)
            .group_open_suffix_span()
        else {
            return;
        };
        self.write_space();
        self.write_text(span.slice(self.source()).trim_start());
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
            Some(if_branch_upper_bound(command, 0, source)),
        )?;
        for (index, (condition, body)) in command.elif_branches.iter().enumerate() {
            self.write_text(" elif ");
            self.format_inline_stmts(condition)?;
            self.write_space();
            self.format_brace_group(
                body,
                Some(if_branch_upper_bound(command, index + 1, source)),
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
                self.format_do_done_body(&command.body, command.span, "done")?;
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
                self.format_do_done_body(&command.body, command.span, "done")?;
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
                self.format_do_done_body(&command.body, command.span, "done")?;
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
                self.format_do_done_body(&command.body, command.span, "done")?;
            }
        }
        Ok(())
    }

    fn format_select(&mut self, command: &SelectCommand) -> Result<()> {
        self.write_text("select ");
        self.write_text(command.variable.as_ref());
        self.write_text(" in");
        self.write_word_list_preserving_breaks(&command.words);
        self.format_do_done_body(&command.body, command.span, "done")?;
        Ok(())
    }

    fn format_while(&mut self, command: &WhileCommand) -> Result<()> {
        self.write_text("while ");
        self.format_inline_stmts(&command.condition)?;
        self.format_do_done_body(&command.body, command.span, "done")
    }

    fn format_until(&mut self, command: &UntilCommand) -> Result<()> {
        self.write_text("until ");
        self.format_inline_stmts(&command.condition)?;
        self.format_do_done_body(&command.body, command.span, "done")
    }

    fn format_case(&mut self, command: &CaseCommand) -> Result<()> {
        self.write_text("case ");
        self.write_word(&command.word);
        self.write_text(" in");
        self.write_case_open_suffix(command);
        if self.options().compact_layout() {
            for item in &command.cases {
                self.write_space();
                self.format_case_item(
                    item,
                    case_item_body_upper_bound(item, command.span.end.offset),
                )?;
            }
            self.write_text(" esac");
        } else {
            for (index, item) in command.cases.iter().enumerate() {
                self.newline();
                if index == 0 && case_has_blank_line_after_in(command, self.source()) {
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
                self.format_case_item(
                    item,
                    case_item_body_upper_bound(item, command.span.end.offset),
                )?;
            }
            if case_has_blank_line_before_esac(command, self.source()) {
                self.newline();
            }
            self.newline();
            self.write_text("esac");
        }
        Ok(())
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
        let Some(in_offset) = header.rfind("in") else {
            return;
        };
        let suffix = &header[in_offset + "in".len()..];
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
            if base_indent > 0 {
                self.with_extra_prefix_indent(base_indent, |formatter| {
                    formatter
                        .emit_leading_comments(&prefix_comments, first_pattern.span.start.line);
                });
            } else {
                self.emit_leading_comments(&prefix_comments, first_pattern.span.start.line);
            }
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
            self.write_pattern(word);
        }
        self.write_text(")");
        let pattern_suffix_comment = self.case_item_pattern_suffix_comment(item, upper_bound);
        if let Some(comment) = &pattern_suffix_comment {
            self.write_space();
            self.write_text(comment);
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
            if base_indent == 0
                && item.body.len() == 1
                && (self.facts().case_item_was_inline_in_source(item)
                    || (pattern_suffix_comment.is_some()
                        && !body_has_later_comments
                        && case_item_body_can_share_terminator(item)
                        && case_item_body_terminator_was_inline_in_source(item))
                    || (!body_has_later_comments
                        && case_item_body_was_inline_without_terminator(item)))
            {
                if pattern_suffix_comment.is_some()
                    && !self.facts().case_item_was_inline_in_source(item)
                {
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
            if case_item_has_blank_line_after_pattern(item, self.source(), first_body_line) {
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
            self.write_space();
            self.write_text(&comment);
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
    ) -> Option<String> {
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
        before.contains(')').then(|| {
            line[comment_start..]
                .trim_end_matches([' ', '\t', '\r'])
                .to_string()
        })
    }

    fn case_item_terminator_suffix_comment(&self, item: &CaseItem) -> Option<String> {
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
        let comment = suffix
            .trim_start_matches([' ', '\t'])
            .trim_end_matches([' ', '\t', '\r']);
        comment.starts_with('#').then(|| comment.to_string())
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

    fn with_extra_prefix_indent<T>(&mut self, levels: usize, f: impl FnOnce(&mut Self) -> T) -> T {
        self.indent_level += levels;
        let result = f(self);
        self.indent_level = self.indent_level.saturating_sub(levels);
        result
    }

    fn format_brace_group(&mut self, commands: &StmtSeq, upper_bound: Option<usize>) -> Result<()> {
        let sequence_facts = self.facts().sequence(commands, upper_bound);
        let should_inline = sequence_facts.group_open_suffix_span().is_none()
            && self.facts().group_was_inline_in_source(commands)
            && self.can_inline_group(commands);
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
            && ((self.facts().group_was_inline_in_source(commands)
                && self.can_inline_group(commands))
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
        let init = format_arithmetic_for_init_source(init);
        self.write_text("for ((");
        self.write_text(&init);
        self.write_text(";");
        self.write_text(condition);
        self.write_text(";");
        self.write_text(step);
        self.write_text("))");
        self.format_do_done_body(&command.body, command.span, "done")
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
        }
        Ok(())
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
                    && self.facts().group_was_inline_in_source(commands)
                    && self.can_inline_group(commands);
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
                    && self.facts().group_was_inline_in_source(commands)
                    && self.can_inline_group(commands);
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
                self.write_indent_units(1);
                self.format_inline_stmt(item.stmt)?;
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
                    && self.facts().case_item_was_inline_in_source(item)
                    && !self
                        .facts()
                        .sequence(&item.body, Some(command.span.end.offset))
                        .has_comments()
        })
    }

    fn format_inline_case(&mut self, command: &CaseCommand) -> Result<()> {
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
        if commands.is_empty() {
            return Ok(());
        }

        if self.options().compact_layout() {
            self.write_space();
            self.format_stmt_sequence(commands, upper_bound)
        } else {
            self.newline();
            if preserve_open_blank {
                self.newline();
            }
            self.with_indent(|formatter| formatter.format_stmt_sequence(commands, upper_bound))
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

    fn format_group_with_upper_bound(
        &mut self,
        open: &'static str,
        close: &'static str,
        open_char: char,
        commands: &StmtSeq,
        leading_space: bool,
        upper_bound: Option<usize>,
    ) -> Result<()> {
        if leading_space {
            self.write_space();
        }
        self.write_text(open);
        let open_suffix_span = self
            .facts()
            .sequence(commands, upper_bound)
            .group_open_suffix_span();
        if let Some(span) = open_suffix_span {
            self.write_space();
            self.write_text(span.slice(self.source()).trim_start());
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

        self.format_body_with_upper_bound_and_open_blank(
            commands,
            upper_bound,
            preserve_open_blank,
        )?;
        if preserve_close_blank {
            self.newline();
        }
        self.finish_block(close);
        Ok(())
    }

    fn format_redirect_list(&mut self, redirects: &[Redirect]) {
        for (index, redirect) in redirects.iter().enumerate() {
            if index > 0 {
                self.write_space();
            }
            self.format_redirect(redirect);
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

    fn queue_heredocs(&mut self, redirects: &[Redirect]) {
        let source = self.source();
        for redirect in redirects {
            let Some(heredoc) = redirect.heredoc() else {
                continue;
            };
            let body = if !self.options.simplify()
                && !self.options.minify()
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
        if assignment_has_multiline_literal_source(assignment, source) {
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

    fn write_multiline_compound_literal_assignment(&mut self, assignment: &Assignment) {
        let raw = assignment.span.slice(self.source());
        let Some((head, tail)) = raw.split_once('\n') else {
            self.write_text(raw);
            return;
        };

        self.write_text(head);
        self.newline();
        self.write_indent_units(1);
        self.write_verbatim(tail.trim_start_matches([' ', '\t']));
    }

    fn can_inline_body(&self, commands: &StmtSeq, enclosing_span: Span) -> bool {
        let [command] = commands.as_slice() else {
            return false;
        };
        if matches!(command.terminator, Some(StmtTerminator::Background(_)))
            || !self.can_inline_stmt(command)
        {
            return false;
        }

        if self
            .facts()
            .sequence(commands, Some(enclosing_span.end.offset))
            .has_comments()
        {
            return false;
        }

        self.options().compact_layout()
            || stmt_span(command).start.line == enclosing_span.start.line
    }

    fn can_inline_group(&self, commands: &StmtSeq) -> bool {
        let [command] = commands.as_slice() else {
            return false;
        };

        self.can_inline_stmt(command)
            && stmt_span(command).start.line == stmt_span(command).end.line
            && self.can_inline_body(commands, stmt_span(command))
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
        if !group_span.slice(self.source()).contains('\n') {
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
            self.write_indent_units(1);
            self.format_conditional_expr(&expression.right)?;
            return Ok(());
        }
        self.write_space();
        self.format_conditional_expr(&expression.right)
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
    raw.contains(">&$") || raw.contains("<&$") || raw.contains(">&-") || raw.contains("<&-")
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
        AssignmentValue::Scalar(word) => {
            assignment_has_raw_backslash_continuation_literal(assignment, source)
                || word_has_multiline_literal_source(word, source)
        }
        AssignmentValue::Compound(array) => array.elements.iter().any(|element| match element {
            ArrayElem::Sequential(word)
            | ArrayElem::Keyed { value: word, .. }
            | ArrayElem::KeyedAppend { value: word, .. } => {
                word_has_multiline_literal_source(word, source)
            }
        }),
    }
}

fn assignment_has_raw_backslash_continuation_literal(
    assignment: &Assignment,
    source: &str,
) -> bool {
    let raw = assignment.span.slice(source);
    raw.contains("\\\n")
        && !raw.contains("$(")
        && !raw.contains('`')
        && !raw.contains("<(")
        && !raw.contains(">(")
}

fn assignment_source_has_command_substitution(assignment: &Assignment, source: &str) -> bool {
    let raw = assignment.span.slice(source);
    raw.contains("$(") || raw.contains('`') || raw.contains("<(") || raw.contains(">(")
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
    let first_start = source_map
        .first_comment_between(open_end_offset, first_start)
        .unwrap_or(first_start);
    gap_has_blank_line(source, open_end_offset, first_start)
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

fn source_has_blank_line_before_last_keyword(source: &str, span: Span, keyword: &str) -> bool {
    let upper = span.end.offset.min(source.len());
    let lower = span.start.offset.min(upper);
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
    source_has_blank_line_immediately_before_offset(source, lower + keyword_start)
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
        Command::Compound(CompoundCommand::BraceGroup(commands)) => {
            group_attachment_span(commands.as_slice(), source_map, '{', '}')
                .map(|span| span.start.offset)
                .unwrap_or_else(|| stmt_verbatim_span(stmt, source).start.offset)
        }
        Command::Compound(CompoundCommand::Subshell(commands)) => {
            group_attachment_span(commands.as_slice(), source_map, '(', ')')
                .map(|span| span.start.offset)
                .unwrap_or_else(|| stmt_verbatim_span(stmt, source).start.offset)
        }
        _ => stmt_verbatim_span(stmt, source).start.offset,
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

fn case_item_has_blank_line_before(previous: &CaseItem, item: &CaseItem, source: &str) -> bool {
    let Some(start) = previous
        .terminator_span
        .map(|span| span.end.offset)
        .or_else(|| previous.body.last().map(|stmt| stmt_span(stmt).end.offset))
        .or_else(|| {
            previous
                .patterns
                .last()
                .map(|pattern| pattern.span.end.offset)
        })
    else {
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

fn case_item_body_terminator_was_inline_in_source(item: &CaseItem) -> bool {
    let [stmt] = item.body.as_slice() else {
        return false;
    };
    item.terminator_span
        .is_some_and(|span| span.start.line == stmt_format_span(stmt).end.line)
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

fn case_item_has_blank_line_after_pattern(
    item: &CaseItem,
    source: &str,
    first_body_line: usize,
) -> bool {
    let Some(pattern_line) = item.patterns.last().map(|pattern| pattern.span.end.line) else {
        return false;
    };
    let stmt_line = first_body_line;
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
    let Some(start) = last_item
        .terminator_span
        .map(|span| span.end.offset)
        .or_else(|| last_item.body.last().map(|stmt| stmt_span(stmt).end.offset))
    else {
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
    matches!(last.command, Command::Compound(CompoundCommand::Case(_)))
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
            break;
        };
        widths.push(width);
        next_start = end.checked_add(1).filter(|offset| *offset < source.len());
    }

    (widths.len() > 1).then(|| widths.into_iter().max().unwrap_or(0) + 1)
}

fn trailing_comment_tab_indent_adjust(source: &str, comment: &SourceComment<'_>) -> usize {
    let Some((line_start, _)) = line_bounds_for_offset(source, comment.span().start.offset) else {
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
    let normalized = trim_arithmetic_expansion_padding_for_alignment(&collapsed);
    normalized.chars().count() + moved_function_brace_alignment_width(&normalized)
}

fn moved_function_brace_alignment_width(text: &str) -> usize {
    let trimmed = text.trim_end();
    if trimmed
        .strip_prefix("function ")
        .is_some_and(|rest| !rest.trim().is_empty())
    {
        return 1;
    }
    usize::from(trimmed.ends_with("()") && !trimmed.ends_with(" ()"))
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

fn sequence_verbatim_span(statements: &StmtSeq, source: &str) -> Option<Span> {
    statements
        .iter()
        .map(|stmt| stmt_verbatim_span(stmt, source))
        .reduce(|left, right| left.merge(right))
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

fn pipeline_operator_breaks(
    statements: &[&Stmt],
    operators: &[(BinaryOp, Span)],
    source: &str,
) -> Vec<bool> {
    let mut breaks = Vec::with_capacity(operators.len());
    for index in 1..statements.len() {
        let Some((_, operator_span)) = operators.get(index - 1) else {
            continue;
        };
        let previous_end = stmt_span(statements[index - 1]).end.offset;
        let next_start = stmt_span(statements[index]).start.offset;
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

fn if_branch_upper_bound(command: &IfCommand, branch_index: usize, source: &str) -> usize {
    if let Some((start, end)) = if_next_branch_region(command, branch_index, source) {
        branch_prefix_comments(source, start, end)
            .first()
            .map(|comment| comment.offset)
            .unwrap_or(end)
    } else {
        command.span.end.offset
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
    source[start..end]
        .rfind(keyword)
        .map(|offset| start + offset)
}

#[derive(Debug, Clone)]
struct BranchPrefixComment {
    offset: usize,
    text: String,
}

fn branch_prefix_comments(source: &str, start: usize, end: usize) -> Vec<BranchPrefixComment> {
    let start = start.min(end).min(source.len());
    let end = end.min(source.len());
    let Some(slice) = source.get(start..end) else {
        return Vec::new();
    };
    let keyword_indent = line_indent_before_offset(source, end).unwrap_or("");

    let mut comments = Vec::new();
    let mut offset = start;
    for line in slice.split_inclusive('\n') {
        let text = line.trim_end_matches(['\n', '\r']);
        let trimmed = text.trim_start_matches([' ', '\t']);
        let indent = text.len().saturating_sub(trimmed.len());
        if trimmed.starts_with('#') && text.get(..indent) == Some(keyword_indent) {
            comments.push(BranchPrefixComment {
                offset: offset + indent,
                text: trimmed.trim_end_matches([' ', '\t', '\r']).to_string(),
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
