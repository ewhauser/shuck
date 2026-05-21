use std::mem;

use crate::options::{IndentStyle, LineEnding, ResolvedShellFormatOptions};

#[derive(Debug, Clone)]
pub(super) struct PendingHeredoc {
    pub(super) body: String,
    pub(super) delimiter: String,
    pub(super) strip_tabs: bool,
}

pub(super) trait StreamSink {
    fn push_char(&mut self, ch: char);
    fn push_str(&mut self, text: &str);
}

pub(super) struct BufferSink {
    buffer: String,
}

pub(super) struct ShellWriter<S> {
    options: ResolvedShellFormatOptions,
    output: S,
    indent_buffer: String,
    indent_level: usize,
    column: usize,
    line_indent_column: usize,
    line_start: bool,
    pending_heredocs: Vec<PendingHeredoc>,
}

impl ShellWriter<BufferSink> {
    pub(super) fn new_buffer(source: &str, options: &ResolvedShellFormatOptions) -> Self {
        Self::with_output(options, BufferSink::with_capacity(source.len()))
    }

    pub(super) fn with_output_buffer(options: &ResolvedShellFormatOptions, output: String) -> Self {
        Self::with_output(options, BufferSink::new(output))
    }

    pub(super) fn finish_into_string(mut self) -> String {
        self.flush_pending_heredocs();
        self.output.finish_into_string()
    }
}

impl<'source> ShellWriter<CompareSink<'source>> {
    pub(super) fn new_compare(source: &'source str, options: &ResolvedShellFormatOptions) -> Self {
        Self::with_output(options, CompareSink::new(source))
    }

    pub(super) fn finish_matches_source(mut self) -> bool {
        self.flush_pending_heredocs();
        self.output.finish(self.options.line_ending())
    }
}

impl<S> ShellWriter<S>
where
    S: StreamSink,
{
    fn with_output(options: &ResolvedShellFormatOptions, output: S) -> Self {
        Self {
            options: options.clone(),
            output,
            indent_buffer: String::new(),
            indent_level: 0,
            column: 0,
            line_indent_column: 0,
            line_start: true,
            pending_heredocs: Vec::new(),
        }
    }

    pub(super) fn indent_level(&self) -> usize {
        self.indent_level
    }

    pub(super) fn column(&self) -> usize {
        self.column
    }

    pub(super) fn line_indent_column(&self) -> usize {
        self.line_indent_column
    }

    pub(super) fn line_start(&self) -> bool {
        self.line_start
    }

    pub(super) fn set_line_start(&mut self, line_start: bool) {
        self.line_start = line_start;
    }

    pub(super) fn mark_line_started_with_zero_indent(&mut self) {
        self.line_indent_column = 0;
        self.line_start = false;
    }

    pub(super) fn push_indent(&mut self, levels: usize) {
        self.indent_level += levels;
    }

    pub(super) fn pop_indent(&mut self, levels: usize) {
        self.indent_level = self.indent_level.saturating_sub(levels);
    }

    pub(super) fn line_ending(&self) -> &'static str {
        match self.options.line_ending() {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
        }
    }

    pub(super) fn indent_column_for_level(&self, level: usize) -> usize {
        if self.options.minify() {
            return 0;
        }
        self.options.indent_columns(level)
    }

    pub(super) fn write_indent_units(&mut self, levels: usize) {
        if levels == 0 {
            return;
        }

        if self.line_start {
            self.write_indent();
        }

        self.write_indent_columns(self.indent_column_for_level(levels));
    }

    pub(super) fn write_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        let mut remaining = text;
        while !remaining.is_empty() {
            if self.line_start && !remaining.starts_with('\n') {
                self.write_indent();
            }

            let (line, next, had_newline) = super::split_first_line_including_newline(remaining);
            self.push_output_str(line);
            self.line_start = had_newline;
            if had_newline {
                remaining = next;
            } else {
                break;
            }
        }
    }

    pub(super) fn write_verbatim(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.push_output_str(text);
        self.line_start = text.ends_with('\n');
    }

    pub(super) fn write_indent(&mut self) {
        if !self.line_start || self.indent_level == 0 || self.options.minify() {
            return;
        }

        self.write_indent_columns(self.indent_column_for_level(self.indent_level));
    }

    pub(super) fn write_indent_to_column(&mut self, column: usize) {
        if !self.line_start || column == 0 || self.options.minify() {
            return;
        }

        self.write_indent_columns(column);
    }

    pub(super) fn write_indent_columns(&mut self, columns: usize) {
        let mut indent = mem::take(&mut self.indent_buffer);
        indent.clear();
        self.options.push_indent_columns(&mut indent, columns);
        self.push_output_str(&indent);
        self.indent_buffer = indent;

        self.line_indent_column = self.column;
        self.line_start = false;
    }

    pub(super) fn write_space(&mut self) {
        if self.line_start {
            return;
        }
        self.push_output_char(' ');
    }

    pub(super) fn write_spaces(&mut self, count: usize) {
        for _ in 0..count {
            self.write_space();
        }
    }

    pub(super) fn queue_heredoc(&mut self, heredoc: PendingHeredoc) {
        self.pending_heredocs.push(heredoc);
    }

    pub(super) fn flush_pending_heredocs(&mut self) {
        let pending = mem::take(&mut self.pending_heredocs);
        for heredoc in pending {
            self.push_output_str(self.line_ending());
            self.line_start = true;
            if heredoc.strip_tabs {
                self.write_indented_heredoc_text(&heredoc.body);
            } else {
                self.write_verbatim(&heredoc.body);
            }
            if !heredoc.body.is_empty()
                && !heredoc.body.ends_with('\n')
                && !heredoc.body.ends_with('\r')
            {
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

    pub(super) fn newline(&mut self) {
        self.flush_pending_heredocs();
        self.push_output_str(self.line_ending());
        self.line_start = true;
    }

    pub(super) fn line_continuation(&mut self) {
        // A backslash only escapes the following LF, so CRLF here would change
        // the command structure by leaving the carriage return behind. It also
        // does not terminate a command header, so pending heredocs must remain
        // queued until the next real line break.
        self.push_output_str(" \\\n");
        self.line_start = true;
    }

    pub(super) fn write_line_breaks(&mut self, count: usize) {
        for _ in 0..count {
            self.newline();
        }
    }

    pub(super) fn push_raw_str(&mut self, text: &str) {
        self.push_output_str(text);
    }

    fn write_indented_heredoc_text(&mut self, text: &str) {
        let indent_level = self.indent_level.saturating_add(1);
        let prefix = self.options.indent_prefix(indent_level);
        let base_tabs = if matches!(self.options.indent_style(), IndentStyle::Tab) {
            text.lines()
                .filter(|line| !line.is_empty())
                .map(|line| line.bytes().take_while(|byte| *byte == b'\t').count())
                .min()
                .unwrap_or(0)
        } else {
            0
        };
        let mut rest = text;
        while !rest.is_empty() {
            let (line, next, _) = super::split_first_line_including_newline(rest);
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
}

impl BufferSink {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            buffer: String::with_capacity(capacity),
        }
    }

    fn new(buffer: String) -> Self {
        Self { buffer }
    }

    fn finish_into_string(self) -> String {
        self.buffer
    }
}

impl StreamSink for BufferSink {
    fn push_char(&mut self, ch: char) {
        self.buffer.push(ch);
    }

    fn push_str(&mut self, text: &str) {
        self.buffer.push_str(text);
    }
}

pub(super) struct CompareSink<'source> {
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

impl StreamSink for CompareSink<'_> {
    fn push_char(&mut self, ch: char) {
        CompareSink::push_char(self, ch);
    }

    fn push_str(&mut self, text: &str) {
        CompareSink::push_str(self, text);
    }
}
