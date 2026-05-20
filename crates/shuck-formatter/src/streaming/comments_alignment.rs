use super::*;

impl<'source, 'facts> ShellRenderer<'source, 'facts> {
    pub(super) fn write_comment(&mut self, comment: &SourceComment<'_>) {
        self.write_text(comment.text());
    }

    pub(super) fn emit_leading_comments(
        &mut self,
        comments: &[SourceComment<'_>],
        next_line: usize,
    ) {
        for (index, comment) in comments.iter().enumerate() {
            self.write_comment(comment);
            let target_line = comments
                .get(index + 1)
                .map(SourceComment::line)
                .unwrap_or(next_line);
            self.write_line_breaks(line_gap_break_count(comment.line(), target_line));
        }
    }

    pub(super) fn emit_pipeline_leading_comments_after_operator(
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

    pub(super) fn emit_trailing_comments_for_stmt(&mut self, comments: &[SourceComment<'_>]) {
        for comment in comments {
            let current_code_column = self.column().saturating_sub(self.line_indent_column());
            let padding = trailing_comment_padding(
                self.source(),
                self.source_map(),
                comment,
                current_code_column,
                self.line_indent_column(),
            );
            self.write_spaces(padding);
            self.write_comment(comment);
        }
    }

    pub(super) fn emit_dangling_comments(&mut self, comments: &[SourceComment<'_>]) {
        self.emit_dangling_comments_after(comments, None);
    }

    pub(super) fn emit_dangling_comments_after(
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

    pub(super) fn stmt_rendered_end_line(&self, stmt: &Stmt) -> usize {
        stmt_rendered_end_line_after_format(
            stmt,
            self.source(),
            self.source_map(),
            self.facts().stmt(stmt).rendered_end_line(),
        )
    }

    pub(super) fn stmt_sequence_next_start_line(
        &self,
        statements: &StmtSeq,
        index: usize,
        attachments: Option<&crate::facts::SequenceFacts<'source>>,
    ) -> usize {
        attachments
            .map(|attachment| attachment.first_rendered_line_for(index + 1))
            .unwrap_or_else(|| {
                stmt_render_start_line(
                    &statements[index + 1],
                    self.source(),
                    self.source_map(),
                    self.options(),
                )
            })
    }

    pub(super) fn maybe_preserve_dangling_comment_outdent(&mut self, comment: &SourceComment<'_>) {
        if !self.line_start() {
            return;
        }
        if comment_precedes_close_keyword_at_same_indent(self.source(), self.source_map(), comment)
        {
            let close_indent_column =
                self.indent_column_for_level(self.indent_level().saturating_sub(1));
            if close_indent_column == 0 {
                self.writer.mark_line_started_with_zero_indent();
            } else {
                self.write_indent_to_column(close_indent_column);
            }
        }
    }

    pub(super) fn write_close_suffix_after_span(&mut self, close_span: Option<Span>) {
        let Some(comment) = close_span.and_then(|span| self.close_suffix_comment_after_span(span))
        else {
            return;
        };
        self.write_comment_with_padding(&comment, close_suffix_comment_padding);
    }

    pub(super) fn write_comment_with_padding(
        &mut self,
        comment: &SourceComment<'_>,
        padding_for: impl FnOnce(&str, &SourceMap<'_>, &SourceComment<'_>, usize, usize) -> usize,
    ) {
        let current_code_column = self.column().saturating_sub(self.line_indent_column());
        let padding = padding_for(
            self.source(),
            self.source_map(),
            comment,
            current_code_column,
            self.line_indent_column(),
        );
        self.write_spaces(padding);
        self.write_comment(comment);
    }

    pub(super) fn write_suffix_comment_after_span(&mut self, span: Span, nudge_aligned: bool) {
        let Some(comment) = self.suffix_comment_from_span(span) else {
            self.write_space();
            self.write_text(span.slice(self.source()).trim_start());
            return;
        };
        let current_code_column = self.column().saturating_sub(self.line_indent_column());
        let mut padding = trailing_comment_padding(
            self.source(),
            self.source_map(),
            &comment,
            current_code_column,
            self.line_indent_column(),
        );
        if nudge_aligned
            && trailing_comment_alignment_column(
                self.source(),
                self.source_map(),
                &comment,
                self.line_indent_column(),
            )
            .is_some()
        {
            padding += 1;
        }
        self.write_spaces(padding);
        self.write_comment(&comment);
    }

    pub(super) fn suffix_comment_from_span(&self, span: Span) -> Option<SourceComment<'source>> {
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

    pub(super) fn close_suffix_comment_after_span(
        &self,
        close_span: Span,
    ) -> Option<SourceComment<'source>> {
        self.source_map().suffix_comment_after_span(close_span)
    }
}
