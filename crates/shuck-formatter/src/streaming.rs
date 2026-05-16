use std::fmt::Write as _;
use std::mem;

use shuck_ast::{
    AlwaysCommand, AnonymousFunctionCommand, ArithmeticCommand, ArithmeticForCommand, Assignment,
    AssignmentValue, BinaryCommand, BinaryOp, BuiltinCommand, CaseCommand, CaseItem, Command,
    CompoundCommand, ConditionalBinaryExpr, ConditionalCommand, ConditionalExpr,
    ConditionalParenExpr, ConditionalUnaryExpr, ConditionalUnaryOp, CoprocCommand, DeclClause,
    DeclOperand, File, ForCommand, ForSyntax, ForeachCommand, ForeachSyntax, FunctionDef,
    IfCommand, IfSyntax, Pattern, Redirect, RedirectKind, RepeatCommand, RepeatSyntax,
    SelectCommand, SimpleCommand, Span, Stmt, StmtSeq, StmtTerminator, TimeCommand, UntilCommand,
    VarRef, WhileCommand, Word,
};
use shuck_format::{IndentStyle, LineEnding};

use crate::Result;
use crate::command::{
    binary_operator, case_terminator, command_format_span, group_attachment_span,
    line_gap_break_count, multiline_compound_assignment_lines, render_assignment_head_to_buf,
    render_assignment_with_facts_to_buf, render_background_operator, render_var_ref_to_buf,
    stmt_render_start_line, stmt_span, stmt_verbatim_span,
};
use crate::comments::{SourceComment, SourceMap};
use crate::facts::FormatterFacts;
use crate::options::ResolvedShellFormatOptions;
use crate::word::{
    heredoc_delimiters_in_rendered_line, render_arithmetic_expr_to_buf, render_heredoc_body_to_buf,
    render_pattern_syntax_to_buf, render_word_syntax_with_facts_to_buf,
};

enum StreamOutput<'source> {
    Buffer(String),
    Compare(CompareSink<'source>),
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
    indent_level: usize,
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
    list_continuation_depth: usize,
    column: usize,
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
            list_continuation_depth: 0,
            column: 0,
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
        if ch == '\n' {
            self.column = 0;
        } else if ch != '\r' {
            self.column += 1;
        }
    }

    fn push_output_str(&mut self, text: &str) {
        self.output.push_str(text);
        for ch in text.chars() {
            if ch == '\n' {
                self.column = 0;
            } else if ch != '\r' {
                self.column += 1;
            }
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

    fn with_indent<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.indent_level += 1;
        let result = f(self);
        self.indent_level = self.indent_level.saturating_sub(1);
        result
    }

    fn with_list_continuation<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.list_continuation_depth += 1;
        let result = f(self);
        self.list_continuation_depth = self.list_continuation_depth.saturating_sub(1);
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
        if scratch.contains('\n') && scratch.contains("<<") {
            self.write_heredoc_aware_multiline_text(&scratch);
        } else {
            self.write_text(&scratch);
        }
        self.restore_scratch_buffer(scratch);
    }

    fn write_heredoc_aware_multiline_text(&mut self, text: &str) {
        let mut pending_heredocs = Vec::new();
        let mut active_heredoc = None;
        for (index, line) in text.split('\n').enumerate() {
            if index > 0 {
                self.push_output_str(self.line_ending());
                self.line_start = true;
            }

            if line.is_empty() {
                continue;
            }

            if let Some(delimiter) = active_heredoc.as_deref() {
                self.write_verbatim(line);
                if line == delimiter {
                    active_heredoc = pop_next_heredoc_delimiter(&mut pending_heredocs);
                }
                continue;
            }

            let literal_line = line.trim_start_matches('\t');
            if index > 0
                && matches!(self.options.indent_style(), IndentStyle::Tab)
                && literal_line.starts_with(' ')
            {
                self.write_verbatim(literal_line);
                continue;
            }

            if index > 0 && line.starts_with('"') {
                self.write_verbatim(line);
                continue;
            }

            self.write_text(line);
            pending_heredocs.extend(heredoc_delimiters_in_rendered_line(line));
            active_heredoc = pop_next_heredoc_delimiter(&mut pending_heredocs);
        }
    }

    fn write_multiline_text_with_verbatim_continuations(&mut self, text: &str) {
        let Some((first, rest)) = text.split_once('\n') else {
            self.write_text(text);
            return;
        };

        self.write_text(first.trim_end_matches('\r'));
        self.push_output_str(self.line_ending());
        self.line_start = true;
        self.write_verbatim(rest);
    }

    fn write_multiline_text_preserving_space_prefixed_lines(&mut self, text: &str) {
        for (index, line) in text.split_inclusive('\n').enumerate() {
            let literal_line = line.trim_start_matches('\t');
            if index > 0
                && matches!(self.options.indent_style(), IndentStyle::Tab)
                && literal_line.starts_with(' ')
            {
                self.write_verbatim(literal_line);
            } else {
                self.write_text(line);
            }
        }
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

        self.line_start = false;
    }

    fn write_space(&mut self) {
        if self.line_start {
            return;
        }
        self.push_output_char(' ');
    }

    fn write_spaces(&mut self, count: usize) {
        for _ in 0..count {
            self.write_space();
        }
    }

    fn flush_pending_heredocs(&mut self) {
        let pending = mem::take(&mut self.pending_heredocs);
        for heredoc in pending {
            self.push_output_str(self.line_ending());
            self.line_start = true;
            if heredoc.strip_tabs {
                self.write_tab_stripped_heredoc_body(&heredoc.body, heredoc.indent_level);
            } else {
                self.write_verbatim(&heredoc.body);
            }
            if heredoc_body_needs_separator(&heredoc.body) {
                self.push_output_str(self.line_ending());
                self.line_start = true;
            }
            if heredoc.strip_tabs {
                self.write_exact_indent(heredoc.indent_level);
            }
            self.write_verbatim(&heredoc.delimiter);
        }
    }

    fn write_tab_stripped_heredoc_body(&mut self, body: &str, indent_level: usize) {
        for (index, line) in body.split_inclusive('\n').enumerate() {
            if index > 0 {
                self.line_start = true;
            }
            if !line.is_empty() {
                self.write_exact_indent(indent_level + 1);
                self.write_verbatim(line.trim_start_matches('\t'));
            }
        }
    }

    fn write_exact_indent(&mut self, levels: usize) {
        if !self.line_start || levels == 0 || self.options.minify() {
            return;
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

        self.line_start = false;
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
        let raw = word.span.slice(self.source());
        if scratch.contains('\n') && scratch == raw {
            self.write_verbatim(&scratch);
        } else if scratch.contains('\n') && scratch.contains("<<") {
            self.write_heredoc_aware_multiline_text(&scratch);
        } else if scratch.contains('\n') {
            self.write_multiline_text_preserving_space_prefixed_lines(&scratch);
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
        if multiline_compound_assignment_lines(assignment, self.source()).is_some() {
            self.write_multiline_compound_assignment(assignment);
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
        let raw = assignment.span.slice(self.source());
        if scratch.contains('\n') && scratch == raw {
            self.write_multiline_text_with_verbatim_continuations(&scratch);
        } else if scratch.contains('\n') && scratch.contains("<<") {
            self.write_heredoc_aware_multiline_text(&scratch);
        } else if scratch.contains('\n') {
            self.write_multiline_text_preserving_space_prefixed_lines(&scratch);
        } else {
            self.write_text(&scratch);
        }
        self.restore_scratch_buffer(scratch);
    }

    fn write_multiline_compound_assignment(&mut self, assignment: &Assignment) {
        let source = self.source();
        let Some((first_line, remaining_lines)) =
            compound_assignment_multiline_parts(assignment, source)
        else {
            self.write_assignment(assignment);
            return;
        };

        self.write_assignment_head(assignment);
        self.write_text("(");
        if let Some(first_line) = first_line {
            self.write_text(&first_line);
        }
        if !remaining_lines.is_empty() {
            self.newline();
            self.with_indent(|formatter| {
                for (index, line) in remaining_lines.iter().enumerate() {
                    if index > 0 {
                        formatter.newline();
                    }
                    formatter.write_text(line);
                }
            });
        }
        if !compound_assignment_closes_after_last_element(assignment, source) {
            self.newline();
        }
        self.write_text(")");
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

    fn emit_trailing_comments(
        &mut self,
        comments: &[SourceComment<'_>],
        _previous_end: Option<usize>,
    ) {
        for comment in comments {
            if self.comment_padding_is_in_alignment_run(comment)
                && let Some(target_column) = self.aligned_comment_target_column(comment)
            {
                self.write_spaces(target_column.saturating_sub(self.column).max(1));
            } else {
                self.write_space();
            }
            self.write_comment(comment);
        }
    }

    fn emit_dangling_comments(&mut self, comments: &[SourceComment<'_>]) {
        for (index, comment) in comments.iter().enumerate() {
            self.newline();
            self.write_comment(comment);
            if let Some(next) = comments.get(index + 1) {
                self.write_line_breaks(line_gap_break_count(comment.line(), next.line()));
            }
        }
    }

    fn write_reindented_verbatim_block(&mut self, text: &str) {
        for (index, line) in text
            .trim_end_matches(&['\r', '\n'][..])
            .split('\n')
            .enumerate()
        {
            if index > 0 {
                self.newline();
            }
            if !line.is_empty() {
                self.write_text(line.trim_start());
            }
        }
    }

    fn format_stmt_sequence(
        &mut self,
        statements: &StmtSeq,
        upper_bound: Option<usize>,
    ) -> Result<()> {
        self.format_stmt_sequence_skipping_leading_before(statements, upper_bound, None)
    }

    fn format_stmt_sequence_skipping_leading_before(
        &mut self,
        statements: &StmtSeq,
        upper_bound: Option<usize>,
        skip_leading_before: Option<usize>,
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

        if attachments
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
                    .filter(|comment| {
                        skip_leading_before
                            .is_none_or(|offset| comment.span().start.offset >= offset)
                    })
                    .filter(|comment| comment.span().end.offset <= span.start.offset)
                    .collect::<Vec<_>>();
                self.emit_leading_comments(
                    &leading,
                    self.facts().stmt(first).render_span().start.line,
                );
            }
            let text = span.slice(source);
            if self.indent_level > 0 && text.lines().count() <= 20 {
                self.write_reindented_verbatim_block(text);
            } else {
                self.write_verbatim(text);
            }
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
                    && let Some(skip_leading_before) = skip_leading_before
                {
                    let leading = attachment
                        .leading_for(index)
                        .iter()
                        .copied()
                        .filter(|comment| comment.span().start.offset >= skip_leading_before)
                        .collect::<Vec<_>>();
                    self.emit_leading_comments(&leading, next_line);
                } else {
                    self.emit_leading_comments(attachment.leading_for(index), next_line);
                }
            }

            self.format_stmt(stmt)?;

            if let Some(attachment) = attachments.as_ref() {
                self.emit_trailing_comments(
                    attachment.trailing_for(index),
                    Some(self.facts().stmt(stmt).render_span().end.offset),
                );
            }

            if index + 1 < statements.len() {
                if matches!(stmt.terminator, Some(StmtTerminator::Background(_))) {
                    if self.facts().background_has_explicit_line_break(stmt) {
                        let current_end = self.facts().stmt(stmt).rendered_end_line();
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
                    let current_end = self.facts().stmt(stmt).rendered_end_line();
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

        if let Some(attachment) = attachments.as_ref() {
            self.emit_dangling_comments(attachment.dangling());
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

        match &stmt.command {
            Command::Compound(CompoundCommand::BraceGroup(commands)) => {
                self.format_brace_group(commands, Some(stmt_span(stmt).end.offset))?;
            }
            Command::Compound(CompoundCommand::Subshell(commands)) => {
                self.format_subshell(commands, Some(stmt_span(stmt).end.offset))?;
            }
            _ => self.format_command(&stmt.command)?,
        }

        if !stmt.redirects.is_empty() && !emit_redirects_first {
            let command_is_wrapped_group = matches!(
                stmt.command,
                Command::Compound(CompoundCommand::BraceGroup(_))
                    | Command::Compound(CompoundCommand::Subshell(_))
            );
            if !command_is_wrapped_group
                && command_span != Span::new()
                && self
                    .source()
                    .get(command_span.end.offset..stmt.redirects[0].span.start.offset)
                    .is_some_and(|between| between.contains('\n'))
            {
                self.line_continuation();
                self.write_indent_units(1);
            } else if !redirect_has_adjacent_numeric_fd(&stmt.redirects[0], self.source()) {
                self.write_space();
            }
            self.format_redirect_list(&stmt.redirects);
        }

        self.queue_heredocs(&stmt.redirects);

        if let Some(StmtTerminator::Background(operator)) = stmt.terminator {
            self.write_space();
            self.write_text(render_background_operator(operator));
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
        for assignment in &command.assignments {
            self.write_command_gap(previous_end, assignment.span.start.offset);
            self.write_assignment(assignment);
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
        if command_gap_has_line_continuation(self.source(), previous_end, next_start) {
            self.line_continuation();
            self.write_indent_units(1);
        } else {
            self.write_space();
        }
    }

    fn write_decl_operand(&mut self, operand: &DeclOperand) {
        match operand {
            DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => self.write_word(word),
            DeclOperand::Name(name) => self.write_var_ref(name),
            DeclOperand::Assignment(assignment) => self.write_assignment(assignment),
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

        for (index, stmt) in statements.iter().enumerate() {
            if index > 0 {
                let (operator, operator_span) = operators
                    .get(index - 1)
                    .copied()
                    .unwrap_or((BinaryOp::Pipe, Span::new()));
                let operator_text = binary_operator(&operator);
                let previous_stmt = statements[index - 1];
                if self.pipeline_operator_has_explicit_line_break(
                    operator_span,
                    previous_stmt,
                    stmt,
                ) {
                    if self.options().binary_next_line() {
                        self.line_continuation();
                        self.with_indent(|formatter| {
                            formatter.write_text(operator_text);
                            formatter.write_space();
                            formatter.format_stmt(stmt)
                        })?;
                    } else {
                        self.write_space();
                        self.write_text(operator_text);
                        self.newline();
                        if self.list_continuation_depth > 0 && index == 1 {
                            self.format_stmt(stmt)?;
                        } else {
                            self.with_indent(|formatter| formatter.format_stmt(stmt))?;
                        }
                    }
                    continue;
                }
                self.write_space();
                self.write_text(operator_text);
                self.write_space();
            }
            self.format_stmt(stmt)?;
        }

        Ok(())
    }

    fn pipeline_operator_has_explicit_line_break(
        &self,
        operator_span: Span,
        previous_stmt: &Stmt,
        next_stmt: &Stmt,
    ) -> bool {
        let previous_span_end = stmt_span(previous_stmt).end.offset;
        let previous_end = if previous_span_end <= operator_span.start.offset {
            previous_span_end
        } else {
            self.facts().stmt(previous_stmt).render_span().end.offset
        };
        let next_start = self.facts().stmt(next_stmt).attachment_span().start.offset;
        source_has_newline_between(self.source(), previous_end, operator_span.start.offset)
            || source_has_newline_between(self.source(), operator_span.end.offset, next_start)
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
                let comment_end = formatter
                    .facts()
                    .stmt(item.stmt)
                    .attachment_span()
                    .start
                    .offset;
                if formatter.emit_branch_leading_comments_between(
                    item.operator_span.end.offset,
                    comment_end,
                ) {
                    formatter.newline();
                }
                formatter.with_list_continuation(|formatter| formatter.format_stmt(item.stmt))
            })?;
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
        let multiline_condition =
            self.condition_starts_after_keyword_continuation("if", &command.condition);
        if multiline_condition && !self.options().compact_layout() {
            self.write_text("if");
            self.format_multiline_condition_header(&command.condition)?;
        } else if self.condition_has_top_level_list_break(&command.condition)
            && !self.options().compact_layout()
        {
            self.write_text("if ");
            let suffix_comment = self.format_multiline_condition_after_keyword(&command.condition);
            self.write_text(self.then_separator(&command.condition));
            if let Some(comment) = suffix_comment {
                self.write_space();
                self.write_text(&comment);
            }
        } else {
            self.write_text("if ");
            self.format_inline_stmts(&command.condition)?;
            if command.elif_branches.is_empty()
                && command.else_branch.is_none()
                && self.can_inline_body(&command.then_branch, command.span)
                && self.if_close_starts_on_body_line(command, &command.then_branch)
            {
                self.write_text(self.then_separator(&command.condition));
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
                self.write_text(self.then_separator(&command.condition));
                self.write_space();
                self.format_inline_stmts(&command.then_branch)?;
                self.write_text("; else ");
                self.format_inline_stmts(else_branch)?;
                self.write_text("; fi");
                return Ok(());
            }

            self.write_text(self.then_separator(&command.condition));
        }

        let then_opener_offset = if let IfSyntax::ThenFi { then_span, .. } = command.syntax {
            self.write_header_suffix_comment_between(
                then_span.end.offset,
                command.then_branch.span.start.offset,
            );
            Some(then_span.end.offset)
        } else {
            None
        };
        let then_upper_bound = if_branch_upper_bound(command, 0, source);
        let mut then_branch_was_inlined_before_else = false;
        let mut then_branch_was_inlined_before_elif = false;
        if !self.options().compact_layout()
            && !command.elif_branches.is_empty()
            && self.body_starts_after_then_on_same_line(&command.then_branch)
            && self.can_inline_else_body(&command.then_branch, then_upper_bound)
            && elif_keyword_offset(command, 0, source).is_some_and(|offset| {
                source_line_break_count_between(
                    source,
                    stmt_seq_content_end(&command.then_branch, source),
                    offset,
                ) == 0
            })
        {
            self.write_space();
            self.format_inline_stmts(&command.then_branch)?;
            then_branch_was_inlined_before_elif = true;
        } else if !self.options().compact_layout()
            && command.elif_branches.is_empty()
            && command.else_branch.is_some()
            && self.body_starts_after_then_on_same_line(&command.then_branch)
            && self.can_inline_else_body(&command.then_branch, then_upper_bound)
            && else_keyword_offset(command, source).is_some_and(|offset| {
                source_line_break_count_between(
                    source,
                    stmt_seq_content_end(&command.then_branch, source),
                    offset,
                ) == 0
            })
        {
            self.write_space();
            self.format_inline_stmts(&command.then_branch)?;
            then_branch_was_inlined_before_else = true;
        } else {
            if let Some(then_opener_offset) = then_opener_offset {
                self.format_body_after_opener_offset(
                    &command.then_branch,
                    Some(then_upper_bound),
                    then_opener_offset,
                )?;
            } else {
                self.format_body_with_upper_bound(
                    &command.then_branch,
                    Some(then_upper_bound),
                    None,
                )?;
            }
        }
        for (index, (condition, body)) in command.elif_branches.iter().enumerate() {
            let mut branch_breaks = 1;
            let mut elif_stays_on_previous_line = false;
            if !self.options().compact_layout()
                && let Some(keyword_offset) = elif_keyword_offset(command, index, source)
            {
                let previous_content_end = if index == 0 {
                    stmt_seq_content_end(&command.then_branch, source)
                } else {
                    stmt_seq_content_end(&command.elif_branches[index - 1].1, source)
                };
                let previous_gap_start = if index == 0 {
                    self.branch_body_gap_start(&command.then_branch)
                } else {
                    self.branch_body_gap_start(&command.elif_branches[index - 1].1)
                };
                let emitted_comments =
                    self.emit_branch_leading_comments_between(previous_content_end, keyword_offset);
                branch_breaks = if emitted_comments {
                    1
                } else {
                    let source_breaks =
                        source_line_break_count_between(source, previous_gap_start, keyword_offset);
                    elif_stays_on_previous_line =
                        index == 0 && then_branch_was_inlined_before_elif && source_breaks == 0;
                    source_breaks.clamp(1, 2)
                };
            }
            let multiline_condition = self
                .condition_starts_after_keyword_continuation("elif", condition)
                && !self.options().compact_layout();
            if self.options().compact_layout() {
                self.write_text("; elif ");
                self.format_inline_stmts(condition)?;
                self.write_text(self.then_separator(condition));
            } else if elif_stays_on_previous_line {
                self.write_text("; elif ");
                self.format_inline_stmts(condition)?;
                self.write_text(self.then_separator(condition));
            } else if multiline_condition {
                self.write_line_breaks(branch_breaks);
                self.write_text("elif");
                self.format_multiline_condition_header(condition)?;
            } else if self.condition_has_top_level_list_break(condition) {
                self.write_line_breaks(branch_breaks);
                self.write_text("elif ");
                let suffix_comment = self.format_multiline_condition_after_keyword(condition);
                self.write_text(self.then_separator(condition));
                if let Some(comment) = suffix_comment {
                    self.write_space();
                    self.write_text(&comment);
                }
            } else {
                self.write_line_breaks(branch_breaks);
                self.write_text("elif ");
                self.format_inline_stmts(condition)?;
                self.write_text(self.then_separator(condition));
            }
            self.write_header_suffix_comment_between(
                condition.span.end.offset,
                body.span.start.offset,
            );
            let branch_upper_bound = if_branch_upper_bound(command, index + 1, source);
            let is_final_branch =
                index + 1 == command.elif_branches.len() && command.else_branch.is_none();
            let followed_by_else =
                index + 1 == command.elif_branches.len() && command.else_branch.is_some();
            let can_inline_before_following_else = !followed_by_else
                || else_keyword_offset(command, source).is_some_and(|offset| {
                    source_line_break_count_between(
                        source,
                        stmt_seq_content_end(body, source),
                        offset,
                    ) == 0
                });
            if !self.options().compact_layout()
                && self.body_starts_after_then_on_same_line(body)
                && self.can_inline_else_body(body, branch_upper_bound)
                && can_inline_before_following_else
                && (!is_final_branch || self.if_close_starts_on_body_line(command, body))
            {
                self.write_space();
                self.format_inline_stmts(body)?;
                if is_final_branch {
                    self.write_text("; fi");
                    return Ok(());
                }
                continue;
            }
            if let Some(then_opener_offset) = branch_then_opener_offset(condition, body, source) {
                self.format_body_after_opener_offset(
                    body,
                    Some(branch_upper_bound),
                    then_opener_offset,
                )?;
            } else {
                self.format_body_with_upper_bound(body, Some(branch_upper_bound), None)?;
            }
        }
        if let Some(body) = &command.else_branch {
            let mut branch_breaks = 1;
            let mut else_stays_on_then_line = false;
            let mut else_body_was_formatted = false;
            if !self.options().compact_layout()
                && let Some(keyword_offset) = else_keyword_offset(command, source)
            {
                let previous_content_end = command
                    .elif_branches
                    .last()
                    .map(|(_, body)| stmt_seq_content_end(body, source))
                    .unwrap_or_else(|| stmt_seq_content_end(&command.then_branch, source));
                let previous_gap_start = command
                    .elif_branches
                    .last()
                    .map(|(_, body)| self.branch_body_gap_start(body))
                    .unwrap_or_else(|| self.branch_body_gap_start(&command.then_branch));
                let emitted_comments =
                    self.emit_branch_leading_comments_between(previous_content_end, keyword_offset);
                branch_breaks = if emitted_comments {
                    1
                } else {
                    let source_breaks =
                        source_line_break_count_between(source, previous_gap_start, keyword_offset);
                    else_stays_on_then_line =
                        then_branch_was_inlined_before_else && source_breaks == 0;
                    source_breaks.clamp(1, 2)
                };
            }
            if self.options().compact_layout() {
                self.write_text("; else");
            } else if else_stays_on_then_line {
                self.write_text("; else");
            } else if self.else_body_starts_on_else_line(command, body)
                && self.can_keep_multiline_assignment_on_else_line(body)
            {
                self.write_line_breaks(branch_breaks);
                self.write_text("else ");
                self.format_stmt(&body[0])?;
                if self.if_close_starts_on_body_line(command, body) {
                    self.write_text("; fi");
                    return Ok(());
                }
                else_body_was_formatted = true;
            } else if self.else_body_starts_on_else_line(command, body)
                && self.can_inline_else_body(body, command.span.end.offset)
            {
                self.write_line_breaks(branch_breaks);
                self.write_text("else ");
                self.format_inline_stmts(body)?;
                self.write_text("; fi");
                return Ok(());
            } else {
                self.write_line_breaks(branch_breaks);
                self.write_text("else");
            }
            if !else_body_was_formatted {
                if !self.options().compact_layout()
                    && let Some(keyword_offset) = else_keyword_offset(command, source)
                {
                    self.format_body_after_opener_offset(
                        body,
                        Some(command.span.end.offset),
                        keyword_offset + "else".len(),
                    )?;
                } else {
                    self.format_body_with_upper_bound(body, Some(command.span.end.offset), None)?;
                }
            }
        }
        if self.options().compact_layout() {
            self.write_text("; fi");
        } else {
            self.write_line_breaks(self.if_close_break_count(command));
            self.write_text("fi");
        }
        Ok(())
    }

    fn if_close_break_count(&self, command: &IfCommand) -> usize {
        let IfSyntax::ThenFi { fi_span, .. } = command.syntax else {
            return 1;
        };
        let body = command
            .else_branch
            .as_ref()
            .or_else(|| command.elif_branches.last().map(|(_, body)| body))
            .unwrap_or(&command.then_branch);
        let current_line = body
            .last()
            .map(|stmt| self.facts().stmt(stmt).rendered_end_line())
            .unwrap_or(body.span.end.line);
        let close_line = self
            .source_map()
            .line_number_for_offset(fi_span.start.offset.min(self.source().len()));
        line_gap_break_count(current_line, close_line)
    }

    fn then_separator(&self, condition: &StmtSeq) -> &'static str {
        if condition.len() == 1
            && matches!(
                condition[0].command,
                Command::Compound(CompoundCommand::Case(_))
            )
        {
            " then"
        } else {
            "; then"
        }
    }

    fn if_close_starts_on_body_line(&self, command: &IfCommand, body: &StmtSeq) -> bool {
        let IfSyntax::ThenFi { fi_span, .. } = command.syntax else {
            return false;
        };
        let body_end = stmt_seq_content_end(body, self.source()).min(self.source().len());
        self.source_map().line_number_for_offset(body_end)
            == self
                .source_map()
                .line_number_for_offset(fi_span.start.offset)
    }

    fn emit_branch_leading_comments_between(&mut self, start: usize, end: usize) -> bool {
        let Some(comment_start) = branch_leading_comment_start(self.source(), start, end) else {
            return false;
        };
        let end = end.min(self.source().len());
        let lines = self.source()[comment_start..end]
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                trimmed.starts_with('#').then_some(trimmed.to_string())
            })
            .collect::<Vec<_>>();
        let comment_indent = source_line_indent_len_at(self.source(), comment_start);
        let keyword_indent = source_line_indent_len_at(self.source(), end);
        let extra_indent = usize::from(comment_indent > keyword_indent);
        let previous_offset = trim_trailing_whitespace_before_offset(self.source(), start);
        let previous_line = self.source_map().line_number_for_offset(previous_offset);
        let comment_line = self.source_map().line_number_for_offset(comment_start);
        let initial_breaks = line_gap_break_count(previous_line, comment_line);
        self.with_extra_prefix_indent(extra_indent, |formatter| {
            for (index, line) in lines.into_iter().enumerate() {
                if !formatter.line_start {
                    if index == 0 {
                        formatter.write_line_breaks(initial_breaks);
                    } else {
                        formatter.newline();
                    }
                }
                formatter.write_text(&line);
            }
        });
        true
    }

    fn branch_body_gap_start(&self, body: &StmtSeq) -> usize {
        body.last()
            .map(|stmt| {
                let facts = self.facts().stmt(stmt);
                let mut end = trim_trailing_whitespace_before_offset(
                    self.source(),
                    facts.attachment_span().end.offset,
                );
                if facts.has_trailing_comment() {
                    end = self
                        .source()
                        .get(end..)
                        .and_then(|tail| tail.find(['\r', '\n']).map(|offset| end + offset))
                        .unwrap_or_else(|| self.source().len());
                }
                end
            })
            .unwrap_or_else(|| stmt_seq_content_end(body, self.source()))
    }

    fn format_multiline_condition_header(&mut self, condition: &StmtSeq) -> Result<()> {
        self.newline();
        self.with_indent(|formatter| formatter.write_multiline_condition_source(condition));
        self.newline();
        self.write_text("then");
        Ok(())
    }

    fn write_multiline_condition_source(&mut self, condition: &StmtSeq) {
        let mut rendered_lines: Vec<String> = Vec::new();
        for raw_line in condition
            .span
            .slice(self.source())
            .trim_end_matches(&['\r', '\n'][..])
            .lines()
        {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }

            let (leading_operator, body) = line
                .strip_prefix("||")
                .map(|rest| ("||", rest))
                .or_else(|| line.strip_prefix("&&").map(|rest| ("&&", rest)))
                .map_or((None, line), |(operator, rest)| {
                    (Some(operator), rest.trim_start())
                });

            if let Some(operator) = leading_operator
                && let Some(previous) = rendered_lines.last_mut()
            {
                let trimmed = previous.trim_end();
                let without_continuation = trimmed.strip_suffix('\\').unwrap_or(trimmed).trim_end();
                *previous = format!("{without_continuation} {operator}");
            }

            if !body.is_empty() {
                rendered_lines.push(body.to_string());
            }
        }

        for (index, line) in rendered_lines.iter().enumerate() {
            if index > 0 {
                self.newline();
                self.write_indent_units(1);
            }
            self.write_text(line);
        }
    }

    fn format_multiline_condition_after_keyword(&mut self, condition: &StmtSeq) -> Option<String> {
        let mut lines: Vec<Option<String>> = Vec::new();
        let mut suffix_comment = None;
        for raw_line in self
            .condition_source_text(condition)
            .trim_end_matches(&['\r', '\n'][..])
            .lines()
        {
            let line = raw_line.trim();
            if line.is_empty() {
                lines.push(None);
                continue;
            }
            if line.starts_with('#') {
                suffix_comment.get_or_insert_with(|| line.to_string());
                lines.push(None);
                continue;
            }

            let (leading_operator, body) = line
                .strip_prefix("||")
                .map(|rest| ("||", rest))
                .or_else(|| line.strip_prefix("&&").map(|rest| ("&&", rest)))
                .map_or((None, line), |(operator, rest)| {
                    (Some(operator), rest.trim_start())
                });

            if let Some(operator) = leading_operator
                && let Some(previous) = lines.iter_mut().rev().find_map(Option::as_mut)
            {
                let trimmed = previous.trim_end();
                let without_continuation = trimmed.strip_suffix('\\').unwrap_or(trimmed).trim_end();
                *previous = format!("{without_continuation} {operator}");
            }

            if !body.is_empty() {
                lines.push(Some(normalize_multiline_conditional_line(body)));
            }
        }

        if let Some(last) = lines
            .iter_mut()
            .rev()
            .find_map(Option::as_mut)
            .filter(|line| !line.is_empty())
        {
            if let Some(without_semicolon) = last.strip_suffix(';') {
                *last = without_semicolon.trim_end().to_string();
            }
        }
        while lines
            .last()
            .is_some_and(|line| line.as_ref().is_none_or(|line| line.trim().is_empty()))
        {
            lines.pop();
        }
        if let Some(last) = lines.iter_mut().rev().find_map(Option::as_mut) {
            if let Some(without_continuation) = last.trim_end().strip_suffix('\\') {
                *last = without_continuation.trim_end().to_string();
            }
        }

        for (index, line) in lines.iter().enumerate() {
            if index == 0 {
                if let Some(line) = line {
                    self.write_text(line);
                }
                continue;
            }
            self.newline();
            if let Some(line) = line {
                self.write_indent_units(1);
                self.write_text(line);
            }
        }
        suffix_comment
    }

    fn condition_has_top_level_list_break(&self, condition: &StmtSeq) -> bool {
        self.condition_source_text(condition)
            .trim_end_matches(&['\r', '\n'][..])
            .contains('\n')
            && (condition
                .iter()
                .any(|stmt| self.stmt_has_top_level_list_break(stmt))
                || condition_source_has_leading_list_operator(
                    self.condition_source_text(condition),
                ))
    }

    fn stmt_has_top_level_list_break(&self, stmt: &Stmt) -> bool {
        let Command::Binary(binary) = &stmt.command else {
            return false;
        };
        self.binary_has_top_level_list_break(binary)
    }

    fn binary_has_top_level_list_break(&self, binary: &BinaryCommand) -> bool {
        matches!(binary.op, BinaryOp::And | BinaryOp::Or)
            && self
                .facts()
                .list_item_has_explicit_line_break(binary.op_span)
            || self.stmt_has_top_level_list_break(&binary.left)
            || self.stmt_has_top_level_list_break(&binary.right)
    }

    fn condition_source_text(&self, condition: &StmtSeq) -> &'source str {
        let start = condition.span.start.offset.min(self.source().len());
        let end = stmt_seq_content_end(condition, self.source())
            .min(self.source().len())
            .max(start);
        &self.source()[start..end]
    }

    fn else_body_starts_on_else_line(&self, command: &IfCommand, body: &StmtSeq) -> bool {
        let Some(first) = body.first() else {
            return false;
        };
        let first_start = self
            .facts()
            .stmt(first)
            .attachment_span()
            .start
            .offset
            .min(self.source().len());
        let line_prefix = self.source()[..first_start]
            .rsplit_once('\n')
            .map_or(&self.source()[..first_start], |(_, line)| line);
        line_prefix.trim_start().starts_with("else ")
            || else_keyword_offset(command, self.source()).is_some_and(|offset| {
                self.source_map().line_number_for_offset(offset)
                    == self.source_map().line_number_for_offset(first_start)
            })
    }

    fn body_starts_after_then_on_same_line(&self, body: &StmtSeq) -> bool {
        let Some(first) = body.first() else {
            return false;
        };
        let first_start = self
            .facts()
            .stmt(first)
            .attachment_span()
            .start
            .offset
            .min(self.source().len());
        let line_prefix = self.source()[..first_start]
            .rsplit_once('\n')
            .map_or(&self.source()[..first_start], |(_, line)| line);
        line_prefix.contains("then ")
    }

    fn condition_starts_after_keyword_continuation(
        &self,
        keyword: &str,
        condition: &StmtSeq,
    ) -> bool {
        let source = self.source();
        let offset = condition.span.start.offset.min(source.len());
        let before_condition = &source[..offset];
        let current_line_prefix = before_condition
            .rsplit_once('\n')
            .map_or(before_condition, |(_, line)| line);
        if !current_line_prefix.trim().is_empty() {
            return false;
        }

        before_condition
            .lines()
            .rev()
            .skip(1)
            .find(|line| !line.trim().is_empty())
            .is_some_and(|line| {
                let trimmed = line.trim();
                trimmed == keyword
                    || trimmed
                        .strip_prefix(keyword)
                        .is_some_and(|rest| rest.trim_end().ends_with('\\'))
            })
    }

    fn write_header_suffix_comment_between(&mut self, start: usize, end: usize) {
        let Some(comment) = self.header_suffix_comment_between(start, end) else {
            return;
        };
        self.write_space();
        self.write_text(&comment);
    }

    fn header_suffix_comment_between(&self, start: usize, end: usize) -> Option<String> {
        let source = self.source();
        let start = start.min(source.len());
        let end = end.min(source.len()).max(start);
        let slice = source.get(start..end)?;
        let first_line = slice
            .find(['\r', '\n'])
            .map_or(slice, |line_end| &slice[..line_end]);
        let comment_start = first_line.find('#')?;
        Some(first_line[comment_start..].trim_end().to_string())
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
            ForSyntax::InDoDone {
                do_span, done_span, ..
            } => {
                if let Some(words) = &command.words {
                    self.write_text(" in");
                    for word in words {
                        self.write_space();
                        self.write_word(word);
                    }
                }
                if self.can_inline_body(&command.body, command.span) {
                    self.write_text("; do ");
                    self.format_inline_stmts(&command.body)?;
                    self.write_text("; done");
                } else {
                    self.write_text("; do");
                    self.write_header_suffix_comment_between(
                        do_span.end.offset,
                        command.body.span.start.offset,
                    );
                    self.format_body_after_opener_offset(
                        &command.body,
                        Some(command.span.end.offset),
                        do_span.end.offset,
                    )?;
                    self.finish_block_after_body("done", &command.body, done_span.start.offset);
                }
            }
            ForSyntax::InDirect { .. } => {
                if let Some(words) = &command.words {
                    self.write_text(" in");
                    for word in words {
                        self.write_space();
                        self.write_word(word);
                    }
                }
                self.write_space();
                self.format_inline_stmts(&command.body)?;
            }
            ForSyntax::ParenDoDone {
                do_span, done_span, ..
            } => {
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
                if self.can_inline_body(&command.body, command.span) {
                    self.write_text("); do ");
                    self.format_inline_stmts(&command.body)?;
                    self.write_text("; done");
                } else {
                    self.write_text("); do");
                    self.write_header_suffix_comment_between(
                        do_span.end.offset,
                        command.body.span.start.offset,
                    );
                    self.format_body_after_opener_offset(
                        &command.body,
                        Some(command.span.end.offset),
                        do_span.end.offset,
                    )?;
                    self.finish_block_after_body("done", &command.body, done_span.start.offset);
                }
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
            ForSyntax::InBrace { .. } => {
                if let Some(words) = &command.words {
                    self.write_text(" in");
                    for word in words {
                        self.write_space();
                        self.write_word(word);
                    }
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
            RepeatSyntax::DoDone { do_span, done_span } => {
                if self.can_inline_body(&command.body, command.span) {
                    self.write_text("; do ");
                    self.format_inline_stmts(&command.body)?;
                    self.write_text("; done");
                } else {
                    self.write_text("; do");
                    self.write_header_suffix_comment_between(
                        do_span.end.offset,
                        command.body.span.start.offset,
                    );
                    self.format_body_with_upper_bound(
                        &command.body,
                        Some(command.span.end.offset),
                        None,
                    )?;
                    self.finish_block_after_body("done", &command.body, done_span.start.offset);
                }
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
            ForeachSyntax::InDoDone {
                do_span, done_span, ..
            } => {
                self.write_text(" in ");
                for (index, word) in command.words.iter().enumerate() {
                    if index > 0 {
                        self.write_space();
                    }
                    self.write_word(word);
                }
                if self.can_inline_body(&command.body, command.span) {
                    self.write_text("; do ");
                    self.format_inline_stmts(&command.body)?;
                    self.write_text("; done");
                } else {
                    self.write_text("; do");
                    self.write_header_suffix_comment_between(
                        do_span.end.offset,
                        command.body.span.start.offset,
                    );
                    self.format_body_with_upper_bound(
                        &command.body,
                        Some(command.span.end.offset),
                        None,
                    )?;
                    self.finish_block_after_body("done", &command.body, done_span.start.offset);
                }
            }
        }
        Ok(())
    }

    fn format_select(&mut self, command: &SelectCommand) -> Result<()> {
        self.write_text("select ");
        self.write_text(command.variable.as_ref());
        self.write_text(" in ");
        for (index, word) in command.words.iter().enumerate() {
            if index > 0 {
                self.write_space();
            }
            self.write_word(word);
        }
        if self.can_inline_body(&command.body, command.span) {
            self.write_text("; do ");
            self.format_inline_stmts(&command.body)?;
            self.write_text("; done");
            return Ok(());
        }
        self.write_text("; do");
        if let Some(opener_offset) = block_opener_offset_before_body(
            command.variable_span.end.offset,
            &command.body,
            self.source(),
            "do",
        ) {
            self.write_header_suffix_comment_between(opener_offset, command.body.span.start.offset);
        }
        self.format_body_with_upper_bound(&command.body, Some(command.span.end.offset), None)?;
        if let Some(done_offset) = block_close_offset_after_body(
            &command.body,
            command.span.end.offset,
            self.source(),
            "done",
        ) {
            self.finish_block_after_body("done", &command.body, done_offset);
        } else {
            self.finish_block("done");
        }
        Ok(())
    }

    fn format_while(&mut self, command: &WhileCommand) -> Result<()> {
        self.write_text("while ");
        self.format_inline_stmts(&command.condition)?;
        if self.can_inline_body(&command.body, command.span) {
            self.write_text("; do ");
            self.format_inline_stmts(&command.body)?;
            self.write_text("; done");
            return Ok(());
        }
        self.write_text("; do");
        let opener_offset =
            loop_body_do_opener_offset(&command.condition, &command.body, self.source());
        if let Some(opener_offset) = opener_offset {
            self.write_header_suffix_comment_between(opener_offset, command.body.span.start.offset);
            self.format_body_after_opener_offset(
                &command.body,
                Some(command.span.end.offset),
                opener_offset,
            )?;
        } else {
            self.format_body_with_upper_bound(&command.body, Some(command.span.end.offset), None)?;
        }
        if let Some(done_offset) = block_close_offset_after_body(
            &command.body,
            command.span.end.offset,
            self.source(),
            "done",
        ) {
            self.finish_block_after_body("done", &command.body, done_offset);
        } else {
            self.finish_block("done");
        }
        Ok(())
    }

    fn format_until(&mut self, command: &UntilCommand) -> Result<()> {
        self.write_text("until ");
        self.format_inline_stmts(&command.condition)?;
        if self.can_inline_body(&command.body, command.span) {
            self.write_text("; do ");
            self.format_inline_stmts(&command.body)?;
            self.write_text("; done");
            return Ok(());
        }
        self.write_text("; do");
        let opener_offset =
            loop_body_do_opener_offset(&command.condition, &command.body, self.source());
        if let Some(opener_offset) = opener_offset {
            self.write_header_suffix_comment_between(opener_offset, command.body.span.start.offset);
            self.format_body_after_opener_offset(
                &command.body,
                Some(command.span.end.offset),
                opener_offset,
            )?;
        } else {
            self.format_body_with_upper_bound(&command.body, Some(command.span.end.offset), None)?;
        }
        if let Some(done_offset) = block_close_offset_after_body(
            &command.body,
            command.span.end.offset,
            self.source(),
            "done",
        ) {
            self.finish_block_after_body("done", &command.body, done_offset);
        } else {
            self.finish_block("done");
        }
        Ok(())
    }

    fn format_case(&mut self, command: &CaseCommand) -> Result<()> {
        self.write_text("case ");
        self.write_word(&command.word);
        self.write_text(" in");
        if self.options().compact_layout() || self.case_command_was_inline_in_source(command) {
            for item in &command.cases {
                self.write_space();
                self.format_inline_case_item(item, Some(command.span.end.offset))?;
            }
            self.write_text(" esac");
        } else {
            for (index, item) in command.cases.iter().enumerate() {
                if index == 0 {
                    self.newline();
                } else {
                    let previous = &command.cases[index - 1];
                    let body_upper_bound =
                        case_item_body_upper_bound(item, Some(command.span.end.offset));
                    let breaks = self
                        .case_item_pattern_prefix_comments(item, body_upper_bound)
                        .first()
                        .map_or_else(
                            || case_item_gap_break_count(previous, item),
                            |comment| {
                                let previous_line = previous
                                    .terminator_span
                                    .map(|span| span.end.line)
                                    .or_else(|| {
                                        previous.body.last().map(|stmt| stmt_span(stmt).end.line)
                                    })
                                    .or_else(|| {
                                        previous
                                            .patterns
                                            .last()
                                            .map(|pattern| pattern.span.end.line)
                                    })
                                    .unwrap_or(1);
                                line_gap_break_count(previous_line, comment.line())
                            },
                        );
                    self.write_line_breaks(breaks);
                }
                self.format_case_item(item, Some(command.span.end.offset))?;
            }
            self.write_line_breaks(self.case_close_break_count(command));
            self.write_text("esac");
        }
        Ok(())
    }

    fn case_command_was_inline_in_source(&self, command: &CaseCommand) -> bool {
        !self
            .case_command_source_through_esac(command)
            .contains('\n')
            && command.cases.iter().all(|item| {
                item.body.len() <= 1
                    && (item.body.is_empty() || self.facts().case_item_was_inline_in_source(item))
            })
    }

    fn case_command_source_through_esac(&self, command: &CaseCommand) -> &'source str {
        let start = command.span.start.offset.min(self.source().len());
        let search_start = command
            .cases
            .last()
            .and_then(|item| item.terminator_span)
            .map_or(command.word.span.end.offset, |span| span.end.offset)
            .min(self.source().len())
            .max(start);
        let span_end = command
            .span
            .end
            .offset
            .min(self.source().len())
            .max(search_start);
        let end = self.source()[search_start..span_end]
            .find("esac")
            .map_or(span_end, |offset| search_start + offset + "esac".len());
        &self.source()[start..end]
    }

    fn case_close_break_count(&self, command: &CaseCommand) -> usize {
        let Some(close_offset) = case_close_offset(command, self.source()) else {
            return 1;
        };
        let current_line = command
            .cases
            .last()
            .and_then(|item| {
                item.terminator_span
                    .map(|span| span.end.line)
                    .or_else(|| {
                        item.body
                            .last()
                            .map(|stmt| self.facts().stmt(stmt).rendered_end_line())
                    })
                    .or_else(|| item.patterns.last().map(|pattern| pattern.span.end.line))
            })
            .unwrap_or(command.span.end.line);
        let close_line = self
            .source_map()
            .line_number_for_offset(close_offset.min(self.source().len()));
        line_gap_break_count(current_line, close_line)
    }

    fn format_inline_case_item(
        &mut self,
        item: &CaseItem,
        upper_bound: Option<usize>,
    ) -> Result<()> {
        let body_upper_bound = case_item_body_upper_bound(item, upper_bound);
        for (index, word) in item.patterns.iter().enumerate() {
            if index > 0 {
                self.write_text(" | ");
            }
            self.write_pattern(word);
        }
        self.write_text(")");

        if item.body.is_empty() {
            self.write_space();
            self.write_text(case_terminator(item.terminator));
            self.write_case_terminator_trailing_comment(item);
            return Ok(());
        }

        let body_was_verbatim = self
            .facts()
            .sequence(&item.body, body_upper_bound)
            .is_ambiguous();
        let mut body = String::new();
        format_stmt_sequence_streaming_to_buf(
            self.source(),
            &item.body,
            self.options(),
            self.facts(),
            body_upper_bound,
            &mut body,
        )?;
        let body = body.trim_end();
        let body = body.strip_suffix(';').map_or(body, str::trim_end);
        self.write_space();
        self.write_text(body);
        if !case_item_body_includes_terminator(item, body_was_verbatim) {
            self.write_space();
            self.write_text(case_terminator(item.terminator));
        }
        self.write_case_terminator_trailing_comment(item);
        Ok(())
    }

    fn format_case_item(&mut self, item: &CaseItem, upper_bound: Option<usize>) -> Result<()> {
        let body_upper_bound = case_item_body_upper_bound(item, upper_bound);
        let base_indent =
            usize::from(!self.options().compact_layout() && self.options().switch_case_indent());
        let pattern_prefix_comments =
            self.case_item_pattern_prefix_comments(item, body_upper_bound);
        let pattern_start = item
            .patterns
            .first()
            .map(|pattern| pattern.span.start.offset);

        if !pattern_prefix_comments.is_empty() && !self.options().compact_layout() {
            self.with_extra_prefix_indent(base_indent, |formatter| {
                formatter.emit_leading_comments(
                    &pattern_prefix_comments,
                    item.patterns.first().map_or_else(
                        || item.body.span.start.line,
                        |pattern| pattern.span.start.line,
                    ),
                );
            });
        }
        if base_indent > 0 {
            self.write_case_prefix(base_indent);
        }
        self.format_case_patterns(item, base_indent);
        self.write_text(")");
        let pattern_comment_was_written = self.write_case_pattern_suffix_comment(item);

        if item.body.is_empty() {
            if pattern_comment_was_written && !self.options().compact_layout() {
                self.newline();
                self.write_case_prefix(base_indent + 1);
            } else {
                self.write_space();
            }
            self.write_text(case_terminator(item.terminator));
            self.write_case_terminator_trailing_comment(item);
        } else if self.options().compact_layout() {
            let body_was_verbatim = self
                .facts()
                .sequence(&item.body, body_upper_bound)
                .is_ambiguous();
            self.write_space();
            self.format_stmt_sequence_skipping_leading_before(
                &item.body,
                body_upper_bound,
                pattern_start,
            )?;
            if !case_item_body_includes_terminator(item, body_was_verbatim) {
                self.write_text("; ");
                self.write_text(case_terminator(item.terminator));
            }
            self.write_case_terminator_trailing_comment(item);
        } else {
            if base_indent == 0
                && item.body.len() == 1
                && self.facts().case_item_was_inline_in_source(item)
                && !pattern_comment_was_written
                && pattern_prefix_comments.is_empty()
            {
                self.write_space();
                self.format_stmt(&item.body[0])?;
                self.write_space();
                self.write_text(case_terminator(item.terminator));
                self.write_case_terminator_trailing_comment(item);
                return Ok(());
            }

            let body_was_verbatim = self
                .facts()
                .sequence(&item.body, body_upper_bound)
                .is_ambiguous();
            self.newline();
            self.with_extra_prefix_indent(base_indent + 1, |formatter| {
                formatter.format_stmt_sequence_skipping_leading_before(
                    &item.body,
                    body_upper_bound,
                    pattern_start,
                )
            })?;
            if case_item_body_includes_terminator(item, body_was_verbatim) {
                self.write_case_terminator_trailing_comment(item);
                return Ok(());
            }
            self.write_line_breaks(self.case_terminator_break_count(item, body_upper_bound));
            self.write_case_prefix(base_indent + 1);
            self.write_text(case_terminator(item.terminator));
            self.write_case_terminator_trailing_comment(item);
        }
        Ok(())
    }

    fn case_terminator_break_count(
        &self,
        item: &CaseItem,
        body_upper_bound: Option<usize>,
    ) -> usize {
        let Some(terminator_span) = item.terminator_span else {
            return 1;
        };
        let current_line = item
            .body
            .last()
            .map(|stmt| {
                self.facts()
                    .sequence(&item.body, body_upper_bound)
                    .dangling()
                    .last()
                    .map(SourceComment::line)
                    .unwrap_or_else(|| self.facts().stmt(stmt).rendered_end_line())
            })
            .or_else(|| item.patterns.last().map(|pattern| pattern.span.end.line))
            .unwrap_or(terminator_span.start.line);
        line_gap_break_count(current_line, terminator_span.start.line)
    }

    fn case_item_pattern_prefix_comments(
        &self,
        item: &CaseItem,
        body_upper_bound: Option<usize>,
    ) -> Vec<SourceComment<'source>> {
        let Some(pattern_start) = item
            .patterns
            .first()
            .map(|pattern| pattern.span.start.offset)
        else {
            return Vec::new();
        };
        self.facts()
            .sequence(&item.body, body_upper_bound)
            .leading_for(0)
            .iter()
            .copied()
            .filter(|comment| comment.span().start.offset < pattern_start)
            .collect()
    }

    fn write_case_pattern_suffix_comment(&mut self, item: &CaseItem) -> bool {
        if self.options().minify() {
            return false;
        }
        let Some(last_pattern) = item.patterns.last() else {
            return false;
        };
        let source = self.source();
        let start = last_pattern.span.end.offset.min(source.len());
        let line_end = source[start..]
            .find(['\r', '\n'])
            .map_or(source.len(), |offset| start + offset);
        let mut end = line_end;
        if let Some(first_body_stmt) = item.body.first() {
            end = end.min(
                self.facts()
                    .stmt(first_body_stmt)
                    .attachment_span()
                    .start
                    .offset,
            );
        }
        if let Some(terminator_span) = item.terminator_span {
            end = end.min(terminator_span.start.offset);
        }
        let end = end.min(line_end).max(start);
        let Some(candidate) = source.get(start..end) else {
            return false;
        };
        let Some(comment_start) = candidate.find('#') else {
            return false;
        };
        self.write_space();
        self.write_verbatim(candidate[comment_start..].trim_end());
        true
    }

    fn write_case_terminator_trailing_comment(&mut self, item: &CaseItem) {
        if self.options().minify() {
            return;
        }
        let Some(terminator_span) = item.terminator_span else {
            return;
        };
        let source = self.source();
        let start = terminator_span.end.offset;
        let Some(line_tail) = source.get(start..).and_then(|tail| {
            let end = tail.find(['\r', '\n']).unwrap_or(tail.len());
            tail.get(..end)
        }) else {
            return;
        };
        let Some(comment_start) = line_tail.find('#') else {
            return;
        };
        self.write_space();
        self.write_verbatim(&line_tail[comment_start..]);
    }

    fn format_case_patterns(&mut self, item: &CaseItem, base_indent: usize) {
        if !self.options().compact_layout()
            && case_patterns_have_line_break(item, self.source())
            && let (Some(first), Some(last)) = (item.patterns.first(), item.patterns.last())
        {
            let start = first.span.start.offset.min(self.source().len());
            let end = last.span.end.offset.min(self.source().len()).max(start);
            let mut wrote_line = false;
            let lines = self.source()[start..end]
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            for line in lines {
                if wrote_line {
                    self.newline();
                    self.write_case_prefix(base_indent + 1);
                }
                self.write_text(&line);
                wrote_line = true;
            }
            if wrote_line {
                return;
            }
        }

        for (index, word) in item.patterns.iter().enumerate() {
            if index > 0 {
                self.write_text(" | ");
            }
            self.write_pattern(word);
        }
    }

    fn comment_padding_is_in_alignment_run(&self, comment: &SourceComment<'_>) -> bool {
        self.comment_padding_is_in_alignment_run_for_line(comment.line())
    }

    fn aligned_comment_target_column(&self, comment: &SourceComment<'_>) -> Option<usize> {
        self.aligned_comment_target_column_for_line(comment.line())
    }

    fn comment_padding_is_in_alignment_run_for_line(&self, line: usize) -> bool {
        line > 1 && self.line_has_inline_comment(line - 1) || self.line_has_inline_comment(line + 1)
    }

    fn aligned_comment_target_column_for_line(&self, line: usize) -> Option<usize> {
        let lines = self.source().lines().collect::<Vec<_>>();
        let mut start = line.saturating_sub(1);
        let mut end = start;

        while start > 0 && line_has_inline_comment_text(lines[start - 1]) {
            start -= 1;
        }
        while end + 1 < lines.len() && line_has_inline_comment_text(lines[end + 1]) {
            end += 1;
        }

        (start..=end)
            .filter_map(|index| inline_comment_code_width(lines[index]))
            .max()
            .map(|width| width + 1)
    }

    fn line_has_inline_comment(&self, line_number: usize) -> bool {
        let Some(line) = self.source().lines().nth(line_number.saturating_sub(1)) else {
            return false;
        };
        line_has_inline_comment_text(line)
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
        if self.should_compact_multiline_brace_group(commands, upper_bound) {
            self.write_text("{ ");
            self.format_stmt(&commands[0])?;
            self.write_text("; }");
            return Ok(());
        }
        self.format_group_with_upper_bound("{", "}", '{', commands, false, upper_bound)
    }

    fn format_subshell(&mut self, commands: &StmtSeq, upper_bound: Option<usize>) -> Result<()> {
        let sequence_facts = self.facts().sequence(commands, upper_bound);
        let should_inline = sequence_facts.group_open_suffix_span().is_none()
            && self.facts().group_was_inline_in_source(commands)
            && self.can_inline_group(commands);
        if should_inline {
            self.write_text("(");
            self.format_inline_stmts(commands)?;
            self.write_text(")");
            return Ok(());
        }
        self.format_group_with_upper_bound("(", ")", '(', commands, false, upper_bound)
    }

    fn format_arithmetic(&mut self, command: &ArithmeticCommand) -> Result<()> {
        self.write_text("((");
        if let Some(expr) = &command.expr_ast {
            let mut scratch = self.take_scratch_buffer();
            render_arithmetic_expr_to_buf(&mut scratch, expr, self.source(), self.options());
            self.write_text(&scratch);
            self.restore_scratch_buffer(scratch);
        } else if let Some(span) = command.expr_span {
            self.write_text(span.slice(self.source()).trim());
        }
        self.write_text("))");
        Ok(())
    }

    fn format_arithmetic_for(&mut self, command: &ArithmeticForCommand) -> Result<()> {
        self.write_text("for ((");
        if command.init_span.is_none() {
            self.write_space();
        }
        self.write_arithmetic_for_segment(command.init_span, command.init_ast.as_ref());
        self.write_text("; ");
        self.write_arithmetic_for_segment(command.condition_span, command.condition_ast.as_ref());
        self.write_text("; ");
        self.write_arithmetic_for_segment(command.step_span, command.step_ast.as_ref());
        self.write_text(")); do");
        self.format_body_after_opener_offset(
            &command.body,
            Some(command.span.end.offset),
            command.right_paren_span.end.offset,
        )?;
        if let Some(done_offset) = block_close_offset_after_body(
            &command.body,
            command.span.end.offset,
            self.source(),
            "done",
        ) {
            self.finish_block_after_body("done", &command.body, done_offset);
        } else {
            self.finish_block("done");
        }
        Ok(())
    }

    fn write_arithmetic_for_segment(
        &mut self,
        span: Option<Span>,
        ast: Option<&shuck_ast::ArithmeticExprNode>,
    ) {
        if let Some(expr) = ast {
            let mut scratch = self.take_scratch_buffer();
            render_arithmetic_expr_to_buf(&mut scratch, expr, self.source(), self.options());
            self.write_text(&scratch);
            self.restore_scratch_buffer(scratch);
        } else if let Some(span) = span {
            self.write_text(span.slice(self.source()).trim());
        }
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
        if command.span.slice(self.source()).contains('\n') {
            self.format_multiline_conditional_source(command);
            return Ok(());
        }

        self.write_text("[[ ");
        self.format_conditional_expr(&command.expression)?;
        let tight_close = self.conditional_needs_tight_close(&command.expression);
        self.write_text(if tight_close { "]]" } else { " ]]" });
        Ok(())
    }

    fn format_multiline_conditional_source(&mut self, command: &ConditionalCommand) {
        for (index, line) in command.span.slice(self.source()).lines().enumerate() {
            if index > 0 {
                self.newline();
                self.write_indent_units(1);
            }
            let line = normalize_multiline_conditional_line(line);
            self.write_text(&line);
        }
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
        self.format_named_function_header(function);
        if self.options().function_next_line() {
            self.newline();
        } else {
            self.write_space();
        }
        self.format_function_body(function.body.as_ref(), function.span.end.offset)
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
        match body {
            Stmt {
                command: Command::Compound(CompoundCommand::BraceGroup(commands)),
                negated: false,
                redirects,
                terminator: None,
                ..
            } if redirects.is_empty() => {
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
            self.format_stmt(stmt)?;
        }
        Ok(())
    }

    fn format_body_with_upper_bound(
        &mut self,
        commands: &StmtSeq,
        upper_bound: Option<usize>,
        opener_line: Option<usize>,
    ) -> Result<()> {
        if commands.is_empty() {
            return Ok(());
        }

        if self.options().compact_layout() {
            self.write_space();
            self.format_stmt_sequence(commands, upper_bound)
        } else {
            let line_breaks = opener_line
                .map(|line| {
                    line_gap_break_count(
                        line,
                        self.facts()
                            .sequence(commands, upper_bound)
                            .first_rendered_line_for(0),
                    )
                })
                .unwrap_or(1);
            self.write_line_breaks(line_breaks);
            self.with_indent(|formatter| formatter.format_stmt_sequence(commands, upper_bound))
        }
    }

    fn format_body_after_opener_offset(
        &mut self,
        commands: &StmtSeq,
        upper_bound: Option<usize>,
        opener_offset: usize,
    ) -> Result<()> {
        if commands.is_empty() {
            return Ok(());
        }

        if self.options().compact_layout() {
            self.write_space();
            self.format_stmt_sequence(commands, upper_bound)
        } else {
            let sequence = self.facts().sequence(commands, upper_bound);
            let first_start = sequence
                .leading_for(0)
                .first()
                .map(|comment| comment.span().start.offset)
                .or_else(|| {
                    commands
                        .first()
                        .map(|stmt| self.facts().stmt(stmt).attachment_span().start.offset)
                })
                .unwrap_or(commands.span.start.offset);
            let line_breaks =
                source_line_break_count_between(self.source(), opener_offset, first_start)
                    .clamp(1, 2);
            self.write_line_breaks(line_breaks);
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

    fn finish_block_after_body(
        &mut self,
        close: &'static str,
        body: &StmtSeq,
        close_offset: usize,
    ) {
        if self.options().compact_layout() {
            self.write_text("; ");
            self.write_text(close);
        } else {
            let current_line = body
                .last()
                .map(|stmt| {
                    self.facts()
                        .sequence(body, Some(close_offset))
                        .dangling()
                        .last()
                        .map(SourceComment::line)
                        .unwrap_or_else(|| self.facts().stmt(stmt).rendered_end_line())
                })
                .unwrap_or(body.span.end.line);
            let close_line = self
                .source_map()
                .line_number_for_offset(close_offset.min(self.source().len()));
            self.write_line_breaks(line_gap_break_count(current_line, close_line));
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
        let sequence_facts = self.facts().sequence(commands, upper_bound);
        let open_suffix_span = sequence_facts.group_open_suffix_span();
        let dangling_tail_line = sequence_facts.dangling().last().map(SourceComment::line);
        if let Some(span) = open_suffix_span {
            self.write_group_open_suffix(span);
        }
        let group_span = group_attachment_span(
            commands.as_slice(),
            self.source_map(),
            open_char,
            close.chars().next().unwrap_or(open_char),
        );
        let opener_line = group_span.map(|span| span.start.line);
        self.format_body_with_upper_bound(commands, upper_bound, opener_line)?;
        if self.options().compact_layout() {
            self.write_text("; ");
            self.write_text(close);
        } else {
            let close_breaks = group_span
                .and_then(|span| {
                    commands.last().map(|last| {
                        let rendered_tail_line = dangling_tail_line
                            .unwrap_or_else(|| self.facts().stmt(last).rendered_end_line());
                        line_gap_break_count(rendered_tail_line, span.end.line)
                    })
                })
                .unwrap_or(1);
            self.write_line_breaks(close_breaks);
            self.write_text(close);
        }
        Ok(())
    }

    fn write_group_open_suffix(&mut self, span: Span) {
        let suffix = span.slice(self.source());
        if let Some(comment_start) = suffix.find('#') {
            self.write_space();
            self.write_text(suffix[comment_start..].trim_end());
        } else {
            self.write_text(suffix);
        }
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

        let mut rendered_explicit_fd = false;
        let adjacent_numeric_fd = redirect_has_adjacent_numeric_fd(redirect, source);
        if let Some(name) = &redirect.fd_var {
            self.write_text("{");
            self.write_text(name.as_str());
            self.write_text("}");
            rendered_explicit_fd = true;
        } else if let Some(fd) = redirect
            .fd
            .filter(|fd| should_render_explicit_fd(*fd, redirect, source))
        {
            self.write_display(fd);
            rendered_explicit_fd = true;
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
        if needs_space_before_target(
            redirect.kind,
            &target,
            options.space_redirects(),
            rendered_explicit_fd || adjacent_numeric_fd,
        ) {
            self.write_space();
        }
        self.write_text(&target);
        self.restore_scratch_buffer(target);
    }

    fn queue_heredocs(&mut self, redirects: &[Redirect]) {
        let source = self.source();
        for redirect in redirects {
            let Some(heredoc) = redirect.heredoc() else {
                continue;
            };
            let body = if heredoc.body.source_backed {
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
            let delimiter = heredoc.delimiter.cooked.to_string();
            self.pending_heredocs.push(PendingHeredoc {
                body,
                delimiter,
                strip_tabs: matches!(redirect.kind, RedirectKind::HereDocStrip),
                indent_level: self.indent_level,
            });
        }
    }

    fn format_standalone_multiline_compound_assignment(
        &mut self,
        assignment: &shuck_ast::Assignment,
    ) -> Result<()> {
        let source = self.source();
        let Some((first_line, remaining_lines)) =
            compound_assignment_multiline_parts(assignment, source)
        else {
            self.write_assignment(assignment);
            return Ok(());
        };

        self.write_assignment_head(assignment);
        self.write_text("(");
        if let Some(first_line) = first_line {
            self.write_text(&first_line);
        }
        if !remaining_lines.is_empty() {
            self.newline();
            self.with_indent(|formatter| {
                for (index, line) in remaining_lines.iter().enumerate() {
                    if index > 0 {
                        formatter.newline();
                    }
                    formatter.write_text(line);
                }
            });
        }
        if !compound_assignment_closes_after_last_element(assignment, source) {
            self.newline();
        }
        self.write_text(")");
        Ok(())
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

    fn can_inline_else_body(&self, commands: &StmtSeq, upper_bound: usize) -> bool {
        let [command] = commands.as_slice() else {
            return false;
        };
        !matches!(command.terminator, Some(StmtTerminator::Background(_)))
            && self.can_inline_stmt(command)
            && stmt_span(command).start.line == stmt_span(command).end.line
            && !self
                .facts()
                .sequence(commands, Some(upper_bound))
                .has_comments()
    }

    fn can_keep_multiline_assignment_on_else_line(&self, commands: &StmtSeq) -> bool {
        let [stmt] = commands.as_slice() else {
            return false;
        };
        let Command::Simple(command) = &stmt.command else {
            return false;
        };
        stmt.redirects.is_empty()
            && stmt.terminator.is_none()
            && command.args.is_empty()
            && command.assignments.len() == 1
            && multiline_compound_assignment_lines(&command.assignments[0], self.source()).is_some()
    }

    fn can_inline_group(&self, commands: &StmtSeq) -> bool {
        let [command] = commands.as_slice() else {
            return false;
        };

        self.can_inline_stmt(command)
            && stmt_span(command).start.line == stmt_span(command).end.line
            && self.can_inline_body(commands, stmt_span(command))
    }

    fn should_compact_multiline_brace_group(
        &self,
        commands: &StmtSeq,
        upper_bound: Option<usize>,
    ) -> bool {
        if self.options().compact_layout()
            || self
                .facts()
                .sequence(commands, upper_bound)
                .group_open_suffix_span()
                .is_some()
        {
            return false;
        }

        let [command] = commands.as_slice() else {
            return false;
        };
        let Some(group_span) =
            group_attachment_span(commands.as_slice(), self.source_map(), '{', '}')
        else {
            return false;
        };
        let first_line =
            stmt_render_start_line(command, self.source(), self.source_map(), self.options());
        let last_line = self.facts().stmt(command).rendered_end_line();
        group_span.start.line == first_line
            && group_span.end.line == last_line
            && group_span.start.line != group_span.end.line
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

fn raw_redirect_source_slice<'a>(redirect: &Redirect, source: &'a str) -> Option<&'a str> {
    let span = redirect.span;
    (span.start.offset < span.end.offset && span.end.offset <= source.len())
        .then(|| span.slice(source))
}

fn should_preserve_raw_redirect(raw: &str) -> bool {
    raw.contains(">&$") || raw.contains("<&$")
}

fn should_render_explicit_fd(fd: i32, redirect: &Redirect, source: &str) -> bool {
    raw_redirect_source_slice(redirect, source).is_some_and(|raw| {
        raw.trim_start()
            .strip_prefix(&fd.to_string())
            .is_some_and(|rest| rest.starts_with(['<', '>']))
    })
}

fn redirect_has_adjacent_numeric_fd(redirect: &Redirect, source: &str) -> bool {
    let start = redirect.span.start.offset.min(source.len());
    let Some(prefix) = source.get(..start) else {
        return false;
    };
    let token = prefix
        .rsplit_once(|ch: char| ch.is_whitespace() || ch == ';' || ch == '&' || ch == '|')
        .map_or(prefix, |(_, token)| token);
    !token.is_empty() && token.chars().all(|ch| ch.is_ascii_digit())
}

fn needs_space_before_target(
    kind: RedirectKind,
    target: &str,
    space_redirects: bool,
    explicit_fd: bool,
) -> bool {
    if target.is_empty() {
        return false;
    }
    if explicit_fd {
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

fn decl_operand_span(operand: &DeclOperand) -> Span {
    match operand {
        DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => word.span,
        DeclOperand::Name(name) => name.span,
        DeclOperand::Assignment(assignment) => assignment.span,
    }
}

fn sequence_verbatim_span(statements: &StmtSeq, source: &str) -> Option<Span> {
    statements
        .iter()
        .map(|stmt| stmt_verbatim_span(stmt, source))
        .reduce(|left, right| left.merge(right))
}

fn pop_next_heredoc_delimiter(pending: &mut Vec<String>) -> Option<String> {
    (!pending.is_empty()).then(|| pending.remove(0))
}

fn case_item_body_includes_terminator(item: &CaseItem, body_was_verbatim: bool) -> bool {
    if !body_was_verbatim {
        return false;
    }
    let Some(terminator_span) = item.terminator_span else {
        return false;
    };
    item.body.span.end.offset >= terminator_span.end.offset
}

fn case_item_body_upper_bound(item: &CaseItem, fallback: Option<usize>) -> Option<usize> {
    item.terminator_span
        .map(|span| span.start.offset)
        .or(fallback)
}

fn case_close_offset(command: &CaseCommand, source: &str) -> Option<usize> {
    let start = command
        .cases
        .last()
        .and_then(|item| item.terminator_span)
        .map_or(command.word.span.end.offset, |span| span.end.offset)
        .min(source.len());
    let end = command.span.end.offset.min(source.len()).max(start);
    source
        .get(start..end)?
        .find("esac")
        .map(|offset| start + offset)
}

fn case_item_gap_break_count(previous: &CaseItem, next: &CaseItem) -> usize {
    let previous_line = previous
        .terminator_span
        .map(|span| span.end.line)
        .or_else(|| previous.body.last().map(|stmt| stmt_span(stmt).end.line))
        .or_else(|| {
            previous
                .patterns
                .last()
                .map(|pattern| pattern.span.end.line)
        })
        .unwrap_or(1);
    let next_line = next
        .patterns
        .first()
        .map(|pattern| pattern.span.start.line)
        .or_else(|| next.body.first().map(|stmt| stmt_span(stmt).start.line))
        .unwrap_or(previous_line + 1);
    line_gap_break_count(previous_line, next_line)
}

fn loop_body_do_opener_offset(condition: &StmtSeq, body: &StmtSeq, source: &str) -> Option<usize> {
    block_opener_offset_before_body(condition.span.end.offset, body, source, "do")
}

fn branch_then_opener_offset(condition: &StmtSeq, body: &StmtSeq, source: &str) -> Option<usize> {
    block_opener_offset_before_body(condition.span.end.offset, body, source, "then")
}

fn block_opener_offset_before_body(
    search_start: usize,
    body: &StmtSeq,
    source: &str,
    opener: &str,
) -> Option<usize> {
    let body_start = body
        .first()
        .map(|stmt| stmt_span(stmt).start.offset)
        .unwrap_or(body.span.start.offset)
        .min(source.len());
    let search_start = search_start.min(body_start);
    source
        .get(search_start..body_start)?
        .rfind(opener)
        .map(|offset| search_start + offset + opener.len())
}

fn block_close_offset_after_body(
    body: &StmtSeq,
    upper_bound: usize,
    source: &str,
    close: &str,
) -> Option<usize> {
    let body_end = body
        .last()
        .map(|stmt| stmt_span(stmt).end.offset)
        .unwrap_or(body.span.end.offset)
        .min(source.len());
    let upper_bound = upper_bound.min(source.len()).max(body_end);
    source
        .get(body_end..upper_bound)?
        .rfind(close)
        .map(|offset| body_end + offset)
}

fn case_patterns_have_line_break(item: &CaseItem, source: &str) -> bool {
    let (Some(first), Some(last)) = (item.patterns.first(), item.patterns.last()) else {
        return false;
    };
    let start = first.span.start.offset.min(source.len());
    let end = last.span.end.offset.min(source.len()).max(start);
    source[start..end].contains('\n')
}

fn condition_source_has_leading_list_operator(source: &str) -> bool {
    source.lines().skip(1).any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("&&") || trimmed.starts_with("||")
    })
}

fn normalize_multiline_conditional_line(line: &str) -> String {
    let line = line.trim().trim_end_matches('\\').trim_end();
    let chars = line.chars().collect::<Vec<_>>();
    let mut normalized = String::with_capacity(line.len());
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '(' {
            normalized.push(ch);
            index += 1;
            while chars.get(index).is_some_and(|next| next.is_whitespace()) {
                index += 1;
            }
            continue;
        }
        if ch.is_whitespace()
            && chars
                .get(index + 1..)
                .and_then(|rest| rest.iter().position(|next| !next.is_whitespace()))
                .is_some_and(|relative| chars[index + 1 + relative] == ')')
        {
            if normalized.ends_with('\\') {
                normalized.push(' ');
            }
            index += 1;
            while chars.get(index).is_some_and(|next| next.is_whitespace()) {
                index += 1;
            }
            continue;
        }
        normalized.push(ch);
        index += 1;
    }
    normalized
}

fn source_line_break_count_between(source: &str, start: usize, end: usize) -> usize {
    let start = start.min(end).min(source.len());
    let end = end.min(source.len());
    source[start..end]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
}

fn trim_trailing_whitespace_before_offset(source: &str, offset: usize) -> usize {
    let mut end = offset.min(source.len());
    while end > 0 {
        let Some((start, ch)) = source[..end].char_indices().next_back() else {
            break;
        };
        if !ch.is_whitespace() {
            break;
        }
        end = start;
    }
    end
}

fn line_has_inline_comment_text(line: &str) -> bool {
    let Some(comment_start) = line.find('#') else {
        return false;
    };
    !line[..comment_start].trim().is_empty()
}

fn inline_comment_code_width(line: &str) -> Option<usize> {
    let comment_start = line.find('#')?;
    let code = line[..comment_start].trim_end();
    (!code.trim().is_empty()).then_some(code.chars().count())
}

fn compound_assignment_closes_after_last_element(assignment: &Assignment, source: &str) -> bool {
    let slice = assignment.span.slice(source);
    let Some(close) = slice.rfind(')') else {
        return false;
    };
    let before_close = &slice[..close];
    let close_line = before_close
        .rsplit_once('\n')
        .map_or(before_close, |(_, line)| line);
    !close_line.trim().is_empty()
}

fn compound_assignment_multiline_parts(
    assignment: &Assignment,
    source: &str,
) -> Option<(Option<String>, Vec<String>)> {
    let AssignmentValue::Compound(_) = &assignment.value else {
        return None;
    };

    let slice = assignment.span.slice(source);
    if !slice.contains('\n') {
        return None;
    }

    let open = slice.find('(')?;
    let close = slice.rfind(')')?;
    if close <= open {
        return None;
    }

    let mut lines = slice[open + 1..close].lines();
    let first = lines
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned);
    let remaining = lines
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    if first.is_none() && remaining.is_empty() {
        None
    } else {
        Some((first, remaining))
    }
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

fn source_has_newline_between(source: &str, start: usize, end: usize) -> bool {
    start <= end
        && source
            .get(start..end)
            .is_some_and(|between| between.contains('\n'))
}

fn source_line_indent_len_at(source: &str, offset: usize) -> usize {
    let offset = offset.min(source.len());
    let line_start = source[..offset]
        .rfind(['\n', '\r'])
        .map_or(0, |index| index + 1);
    let line_end = source[offset..]
        .find(['\n', '\r'])
        .map_or(source.len(), |index| offset + index);
    source[line_start..line_end]
        .chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .count()
}

fn command_gap_has_line_continuation(source: &str, previous_end: usize, next_start: usize) -> bool {
    if previous_end <= next_start
        && source
            .get(previous_end.min(source.len())..next_start.min(source.len()))
            .is_some_and(|between| between.contains('\n'))
    {
        return shfmt_keeps_source_continuation_before(source, next_start, previous_end)
            .unwrap_or(true);
    }

    shfmt_keeps_source_continuation_before(source, next_start, previous_end).unwrap_or(false)
}

fn shfmt_keeps_source_continuation_before(
    source: &str,
    next_start: usize,
    previous_end: usize,
) -> Option<bool> {
    let Some(before_next) = source.get(..next_start.min(source.len())) else {
        return None;
    };
    let trimmed = before_next.trim_end_matches([' ', '\t']);
    let Some(line_end) = trimmed.rfind(['\n', '\r']) else {
        return None;
    };
    let continued_line = &trimmed[..line_end];
    if !continued_line.ends_with('\\') {
        return None;
    }

    let first_non_ws_after_continuation = source[line_end + 1..next_start.min(source.len())]
        .find(|ch| ch != ' ' && ch != '\t' && ch != '\r' && ch != '\n')
        .map_or(next_start.min(source.len()), |offset| line_end + 1 + offset);
    let before_backslash = continued_line[..continued_line.len() - 1]
        .chars()
        .next_back();
    let next_token = source
        .get(next_start.min(source.len())..)
        .and_then(|tail| tail.chars().next());
    let shfmt_keeps_break = before_backslash.is_none_or(char::is_whitespace)
        || matches!(next_token, Some('<' | '>'))
        || (matches!(next_token, Some('-')) && matches!(before_backslash, Some('}' | '"' | '\'')));

    Some(
        shfmt_keeps_break
            && (previous_end > next_start || previous_end <= first_non_ws_after_continuation),
    )
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
    let current_branch_end = if branch_index == 0 {
        stmt_seq_content_end(&command.then_branch, source)
    } else {
        command
            .elif_branches
            .get(branch_index - 1)
            .map(|(_, body)| stmt_seq_content_end(body, source))
            .unwrap_or_else(|| stmt_seq_content_end(&command.then_branch, source))
    };

    if let Some((condition, _)) = command.elif_branches.get(branch_index) {
        let keyword_offset = branch_keyword_offset(
            source,
            current_branch_end,
            condition.span.start.offset,
            "elif",
        )
        .unwrap_or(condition.span.start.offset);
        branch_leading_comment_start(source, current_branch_end, keyword_offset)
            .unwrap_or(keyword_offset)
    } else if let Some(body) = &command.else_branch {
        let keyword_offset =
            branch_keyword_offset(source, current_branch_end, body.span.start.offset, "else")
                .unwrap_or(body.span.start.offset);
        branch_leading_comment_start(source, current_branch_end, keyword_offset)
            .unwrap_or(keyword_offset)
    } else {
        command.span.end.offset
    }
}

fn elif_keyword_offset(command: &IfCommand, branch_index: usize, source: &str) -> Option<usize> {
    let current_branch_end = if branch_index == 0 {
        stmt_seq_content_end(&command.then_branch, source)
    } else {
        stmt_seq_content_end(&command.elif_branches.get(branch_index - 1)?.1, source)
    };
    let condition_start = command.elif_branches.get(branch_index)?.0.span.start.offset;
    branch_keyword_offset(source, current_branch_end, condition_start, "elif")
}

fn else_keyword_offset(command: &IfCommand, source: &str) -> Option<usize> {
    let body = command.else_branch.as_ref()?;
    let current_branch_end = command
        .elif_branches
        .last()
        .map(|(_, body)| stmt_seq_content_end(body, source))
        .unwrap_or_else(|| stmt_seq_content_end(&command.then_branch, source));
    branch_keyword_offset(source, current_branch_end, body.span.start.offset, "else")
}

fn stmt_seq_content_end(commands: &StmtSeq, source: &str) -> usize {
    commands
        .last()
        .map(|stmt| stmt_verbatim_span(stmt, source).end.offset)
        .unwrap_or(commands.span.end.offset)
}

fn branch_leading_comment_start(source: &str, start: usize, end: usize) -> Option<usize> {
    let start = start.min(end).min(source.len());
    let end = end.min(source.len());
    let mut offset = start;
    let mut first_comment = None;
    while offset < end {
        let relative_end = source[offset..end]
            .find('\n')
            .map_or(end - offset, |index| index);
        let line_end = offset + relative_end;
        let line = &source[offset..line_end];
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed == "\\" {
            offset = (line_end + 1).min(end);
            continue;
        }
        if trimmed.starts_with('#') {
            first_comment.get_or_insert(offset);
            offset = (line_end + 1).min(end);
            continue;
        }
        return None;
    }
    first_comment
}

fn branch_keyword_offset(source: &str, start: usize, end: usize, keyword: &str) -> Option<usize> {
    let start = start.min(end).min(source.len());
    let end = end.min(source.len());
    source[start..end]
        .rfind(keyword)
        .map(|offset| start + offset)
}
