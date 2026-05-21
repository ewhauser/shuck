use super::*;

impl<'source, 'facts, S> ShellRenderer<'source, 'facts, S>
where
    S: StreamSink,
{
    pub(super) fn format_if(&mut self, command: &IfCommand) -> Result<()> {
        match command.syntax {
            IfSyntax::ThenFi { .. } => self.format_then_fi_if(command),
            IfSyntax::Brace { .. } => self.format_brace_if(command),
        }
    }

    pub(super) fn format_then_fi_if(&mut self, command: &IfCommand) -> Result<()> {
        let source = self.source();
        let then_span = match command.syntax {
            IfSyntax::ThenFi { then_span, .. } => then_span,
            IfSyntax::Brace { .. } => unreachable!("brace if cannot be formatted as then/fi"),
        };
        let fi_span = command_if_close_span(command, source, self.source_map());
        let layout = self.then_fi_if_layout(command, then_span, fi_span);

        match layout {
            ThenFiIfLayout::RawGroupedCondition { raw_condition } => {
                self.format_raw_grouped_then_fi_if(command, then_span, fi_span, &raw_condition)
            }
            ThenFiIfLayout::SplitCondition => {
                self.format_split_condition_then_fi_if(command, then_span, fi_span)
            }
            layout => self.format_inline_condition_then_fi_if(command, then_span, fi_span, layout),
        }
    }

    pub(super) fn format_raw_grouped_then_fi_if(
        &mut self,
        command: &IfCommand,
        then_span: Span,
        fi_span: Span,
        raw_condition: &str,
    ) -> Result<()> {
        let source = self.source();
        let fi_upper_bound = fi_span.start.offset;
        self.write_text("if");
        self.write_text(raw_condition);
        self.write_text("then");
        let then_upper_bound =
            if_branch_upper_bound(command, 0, source, self.source_map(), self.facts());
        self.format_if_branch_body_after_open(
            &command.then_branch,
            then_upper_bound,
            then_span.end.offset,
        )?;
        if let Some(body) = &command.else_branch {
            if self.if_next_branch_has_blank_line_before_keyword(command, 0, source) {
                self.newline();
            }
            self.newline();
            self.write_text("else");
            self.format_else_branch_body(command, body, fi_upper_bound)?;
        }
        self.finish_multiline_if_close(command, then_upper_bound, fi_span);
        Ok(())
    }

    pub(super) fn format_split_condition_then_fi_if(
        &mut self,
        command: &IfCommand,
        then_span: Span,
        fi_span: Span,
    ) -> Result<()> {
        let source = self.source();
        let fi_upper_bound = fi_span.start.offset;
        self.write_text("if");
        self.newline();
        self.with_indent(|formatter| {
            formatter.format_stmt_sequence(&command.condition, Some(then_span.start.offset))
        })?;
        self.newline();
        self.write_text("then");
        let then_upper_bound =
            if_branch_upper_bound(command, 0, source, self.source_map(), self.facts());
        self.format_if_branch_body_after_open(
            &command.then_branch,
            then_upper_bound,
            then_span.end.offset,
        )?;
        for (index, (condition, body)) in command.elif_branches.iter().enumerate() {
            self.write_if_branch_prefix(command, index, source);
            self.write_elif_header(command, index, condition, body, source)?;
            let body_upper_bound =
                if_branch_upper_bound(command, index + 1, source, self.source_map(), self.facts());
            self.format_if_branch_body_after_keyword(
                body,
                body_upper_bound,
                condition.span.start.offset,
                "then",
            )?;
        }
        if let Some(body) = &command.else_branch {
            self.write_if_branch_prefix(command, command.elif_branches.len(), source);
            self.write_text("else");
            self.format_else_branch_body(command, body, fi_upper_bound)?;
        }
        self.finish_multiline_if_close(command, then_upper_bound, fi_span);
        Ok(())
    }

    pub(super) fn format_inline_condition_then_fi_if(
        &mut self,
        command: &IfCommand,
        then_span: Span,
        fi_span: Span,
        layout: ThenFiIfLayout,
    ) -> Result<()> {
        self.write_text("if ");
        self.format_inline_stmts(&command.condition)?;
        let then_separator = self.then_separator_for_condition(&command.condition);

        match layout {
            ThenFiIfLayout::InlineThen => {
                self.write_text(then_separator);
                self.write_space();
                self.format_inline_stmts(&command.then_branch)?;
                self.write_if_close("; fi", fi_span);
                Ok(())
            }
            ThenFiIfLayout::InlineThenElse => {
                let Some(else_branch) = command.else_branch.as_ref() else {
                    unreachable!("inline then/else layout requires an else branch");
                };
                self.write_text(then_separator);
                self.write_space();
                self.format_inline_stmts(&command.then_branch)?;
                self.write_text("; else ");
                self.format_inline_stmts(else_branch)?;
                self.write_if_close("; fi", fi_span);
                Ok(())
            }
            ThenFiIfLayout::InlineThenMultilineElse => {
                let Some(else_branch) = command.else_branch.as_ref() else {
                    unreachable!("inline then/multiline else layout requires an else branch");
                };
                self.write_text(then_separator);
                self.write_space();
                self.format_inline_stmts(&command.then_branch)?;
                self.write_text("; else");
                self.format_else_branch_body(command, else_branch, fi_span.start.offset)?;
                let then_upper_bound = if_branch_upper_bound(
                    command,
                    0,
                    self.source(),
                    self.source_map(),
                    self.facts(),
                );
                self.finish_multiline_if_close(command, then_upper_bound, fi_span);
                Ok(())
            }
            ThenFiIfLayout::InlineThenNestedIf => {
                self.write_text(then_separator);
                self.write_space();
                self.format_stmt(&command.then_branch[0])?;
                self.write_if_close("; fi", fi_span);
                Ok(())
            }
            ThenFiIfLayout::InlineChain => {
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
                self.write_if_close("; fi", fi_span);
                Ok(())
            }
            ThenFiIfLayout::Expanded(layout) => {
                self.format_expanded_then_fi_if(command, then_span, fi_span, then_separator, layout)
            }
            ThenFiIfLayout::RawGroupedCondition { .. } | ThenFiIfLayout::SplitCondition => {
                unreachable!("non-inline if layout routed to inline emitter")
            }
        }
    }

    pub(super) fn format_expanded_then_fi_if(
        &mut self,
        command: &IfCommand,
        then_span: Span,
        fi_span: Span,
        then_separator: &'static str,
        layout: ExpandedThenFiIfLayout,
    ) -> Result<()> {
        let source = self.source();
        let fi_upper_bound = fi_span.start.offset;
        self.write_text(then_separator);
        let then_upper_bound =
            if_branch_upper_bound(command, 0, source, self.source_map(), self.facts());
        if !self.write_condition_separator_suffix_comment(&command.condition, then_span) {
            self.write_sequence_open_suffix(&command.then_branch, Some(then_upper_bound));
        }
        let preserve_then_open_blank = self
            .facts()
            .sequence(&command.then_branch, Some(then_upper_bound))
            .has_blank_line_after_open();
        self.format_body_with_upper_bound_and_open_blank(
            &command.then_branch,
            Some(then_upper_bound),
            preserve_then_open_blank,
        )?;
        self.write_unmodeled_branch_background_terminator(&command.then_branch, then_upper_bound);
        for (index, (condition, body)) in command.elif_branches.iter().enumerate() {
            if matches!(layout, ExpandedThenFiIfLayout::Compact) {
                self.write_text("; elif ");
                self.format_inline_stmts(condition)?;
                self.write_text(self.then_separator_for_condition(condition));
            } else {
                self.write_if_branch_prefix(command, index, source);
                self.write_elif_header(command, index, condition, body, source)?;
            }
            let body_upper_bound =
                if_branch_upper_bound(command, index + 1, source, self.source_map(), self.facts());
            self.format_if_branch_body_after_keyword(
                body,
                body_upper_bound,
                condition.span.start.offset,
                "then",
            )?;
        }
        if let Some(body) = &command.else_branch {
            match layout {
                ExpandedThenFiIfLayout::Compact => self.write_text("; else"),
                ExpandedThenFiIfLayout::Multiline { inline_else_close } => {
                    self.write_if_branch_prefix(command, command.elif_branches.len(), source);
                    if inline_else_close {
                        self.write_text("else ");
                        self.format_inline_stmts(body)?;
                        self.write_if_close("; fi", fi_span);
                        return Ok(());
                    }
                    self.write_text("else");
                }
            }
            self.format_else_branch_body(command, body, fi_upper_bound)?;
        }
        match layout {
            ExpandedThenFiIfLayout::Compact => self.write_if_close("; fi", fi_span),
            ExpandedThenFiIfLayout::Multiline { .. } => {
                self.finish_multiline_if_close(command, then_upper_bound, fi_span);
            }
        }
        Ok(())
    }

    pub(super) fn finish_multiline_if_close(
        &mut self,
        command: &IfCommand,
        then_upper_bound: usize,
        fi_span: Span,
    ) {
        if self.if_final_branch_has_blank_line_before_fi(command, then_upper_bound) {
            self.newline();
        }
        self.newline();
        self.write_if_close("fi", fi_span);
    }

    pub(super) fn write_if_close(&mut self, close_text: &str, fi_span: Span) {
        self.write_text(close_text);
        self.write_close_suffix_after_span(Some(fi_span));
    }

    pub(super) fn if_final_branch_has_blank_line_before_fi(
        &self,
        command: &IfCommand,
        _then_upper_bound: usize,
    ) -> bool {
        let (branch_index, body) = if let Some(body) = command.else_branch.as_ref() {
            (command.elif_branches.len(), body)
        } else if let Some((index, (_, body))) =
            command.elif_branches.iter().enumerate().next_back()
        {
            (index + 1, body)
        } else {
            (0, &command.then_branch)
        };
        let upper_bound = self.facts().if_branch_upper_bound(command, branch_index);
        !body.is_empty()
            && self
                .facts()
                .sequence(body, Some(upper_bound))
                .has_blank_line_before_close()
    }

    pub(super) fn write_if_branch_prefix(
        &mut self,
        command: &IfCommand,
        branch_index: usize,
        source: &str,
    ) {
        if self.if_next_branch_has_blank_line_before_keyword(command, branch_index, source) {
            self.newline();
        }
        let preserve_blank_after_prefix = self
            .if_branch_prefix_comments_have_blank_line_before_keyword(
                command,
                branch_index,
                source,
            );
        self.emit_branch_prefix_comments(command, branch_index);
        self.newline();
        if preserve_blank_after_prefix {
            self.newline();
        }
    }

    pub(super) fn write_elif_header(
        &mut self,
        command: &IfCommand,
        branch_index: usize,
        condition: &StmtSeq,
        body: &StmtSeq,
        source: &str,
    ) -> Result<()> {
        if condition_keyword_on_previous_non_empty_line(
            condition,
            source,
            self.source_map(),
            "elif",
        ) || elif_condition_has_explicit_statement_break(
            condition,
            body,
            source,
            self.source_map(),
        ) || !self
            .elif_condition_prefix_comments(command, branch_index, condition)
            .is_empty()
        {
            self.write_text("elif");
            self.newline();
            self.with_indent(|formatter| {
                formatter.format_stmt_sequence(condition, Some(body.span.start.offset))
            })?;
            self.newline();
            self.write_text("then");
            Ok(())
        } else {
            self.write_text("elif ");
            self.format_inline_stmts(condition)?;
            self.write_text(self.then_separator_for_condition(condition));
            Ok(())
        }
    }

    pub(super) fn emit_branch_prefix_comments(&mut self, command: &IfCommand, branch_index: usize) {
        let Some((start, end)) = self.facts().if_next_branch_region(command, branch_index) else {
            return;
        };
        let comments = self
            .facts()
            .branch_prefix_facts(start, end)
            .comments()
            .to_vec();
        if comments.is_empty() {
            return;
        }
        let disabled_branch_block = branch_prefix_comments_use_disabled_body_indent(&comments);
        self.newline();
        for (index, comment) in comments.iter().enumerate() {
            if disabled_branch_block {
                self.with_indent(|formatter| formatter.write_text(&comment.text));
            } else {
                self.write_text(&comment.text);
            }
            if let Some(next) = comments.get(index + 1) {
                let line = self.source_map().line_number_for_offset(comment.offset);
                let next_line = self.source_map().line_number_for_offset(next.offset);
                self.write_line_breaks(line_gap_break_count(line, next_line));
            }
        }
    }

    pub(super) fn elif_condition_prefix_comments(
        &self,
        command: &IfCommand,
        branch_index: usize,
        condition: &StmtSeq,
    ) -> Vec<BranchPrefixComment> {
        let Some((_, keyword_offset)) = self.facts().if_next_branch_region(command, branch_index)
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

    pub(super) fn if_next_branch_has_blank_line_before_keyword(
        &self,
        command: &IfCommand,
        branch_index: usize,
        _source: &str,
    ) -> bool {
        self.facts()
            .if_next_branch_region(command, branch_index)
            .is_some_and(|(start, end)| {
                self.facts()
                    .branch_prefix_facts(start, end)
                    .has_blank_line_before_keyword()
            })
    }

    pub(super) fn if_branch_prefix_comments_have_blank_line_before_keyword(
        &self,
        command: &IfCommand,
        branch_index: usize,
        _source: &str,
    ) -> bool {
        self.facts()
            .if_next_branch_region(command, branch_index)
            .is_some_and(|(start, end)| {
                self.facts()
                    .branch_prefix_facts(start, end)
                    .has_blank_line_after_comments()
            })
    }

    pub(super) fn write_sequence_open_suffix(
        &mut self,
        commands: &StmtSeq,
        upper_bound: Option<usize>,
    ) {
        let Some(span) = self
            .facts()
            .sequence(commands, upper_bound)
            .group_open_suffix_span()
        else {
            return;
        };
        self.write_suffix_comment_after_span(span, false);
    }

    pub(super) fn write_condition_separator_suffix_comment(
        &mut self,
        condition: &StmtSeq,
        then_span: Span,
    ) -> bool {
        let Some(comment) = self.condition_separator_suffix_comment(condition, then_span) else {
            return false;
        };
        self.write_comment_with_padding(&comment, trailing_comment_padding);
        true
    }

    pub(super) fn condition_separator_suffix_comment(
        &self,
        condition: &StmtSeq,
        then_span: Span,
    ) -> Option<SourceComment<'source>> {
        let source = self.source();
        let start = condition.last().map(condition_stmt_command_end)?;
        let end = then_span.start.offset.min(source.len());
        if start >= end {
            return None;
        }
        let region = source.get(start..end)?;
        let comment_rel = region.find('#')?;
        let before_comment = region.get(..comment_rel)?;
        if !before_comment
            .chars()
            .all(|ch| matches!(ch, ' ' | '\t' | '\r' | '\n' | ';'))
        {
            return None;
        }
        let comment_start = start + comment_rel;
        let line_end = source
            .get(comment_start..end)?
            .find('\n')
            .map_or(end, |offset| comment_start + offset);
        let comment = source
            .get(comment_start..line_end)?
            .trim_end_matches([' ', '\t', '\r']);
        self.source_map()
            .source_comment_for_offsets(comment_start, comment_start + comment.len())
    }

    pub(super) fn write_unmodeled_branch_background_terminator(
        &mut self,
        body: &StmtSeq,
        upper_bound: usize,
    ) {
        let Some(operator) = unmodeled_branch_background_operator(body, upper_bound, self.source())
        else {
            return;
        };
        self.write_space();
        self.write_text(operator);
    }

    pub(super) fn format_if_branch_body_after_open(
        &mut self,
        body: &StmtSeq,
        upper_bound: usize,
        _open_end_offset: usize,
    ) -> Result<()> {
        let preserve_open_blank = self
            .facts()
            .sequence(body, Some(upper_bound))
            .has_blank_line_after_open();
        self.format_if_branch_body(body, upper_bound, preserve_open_blank)
    }

    pub(super) fn format_if_branch_body_after_keyword(
        &mut self,
        body: &StmtSeq,
        upper_bound: usize,
        keyword_start_offset: usize,
        keyword: &'static str,
    ) -> Result<()> {
        let _ = (keyword_start_offset, keyword);
        let preserve_open_blank = self
            .facts()
            .sequence(body, Some(upper_bound))
            .has_blank_line_after_open();
        self.format_if_branch_body(body, upper_bound, preserve_open_blank)
    }

    pub(super) fn format_else_branch_body(
        &mut self,
        command: &IfCommand,
        body: &StmtSeq,
        fi_upper_bound: usize,
    ) -> Result<()> {
        self.format_if_branch_body_after_keyword(
            body,
            fi_upper_bound,
            command.span.start.offset,
            "else",
        )
    }

    pub(super) fn format_if_branch_body(
        &mut self,
        body: &StmtSeq,
        upper_bound: usize,
        preserve_open_blank: bool,
    ) -> Result<()> {
        self.write_sequence_open_suffix(body, Some(upper_bound));
        self.format_body_with_upper_bound_and_open_blank(
            body,
            Some(upper_bound),
            preserve_open_blank,
        )?;
        self.write_unmodeled_branch_background_terminator(body, upper_bound);
        Ok(())
    }

    pub(super) fn format_brace_if(&mut self, command: &IfCommand) -> Result<()> {
        let source = self.source();
        self.write_text("if ");
        self.format_inline_stmts(&command.condition)?;
        self.write_space();
        self.format_brace_group(
            &command.then_branch,
            Some(if_branch_upper_bound(
                command,
                0,
                source,
                self.source_map(),
                self.facts(),
            )),
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
                    self.facts(),
                )),
            )?;
        }
        if let Some(body) = &command.else_branch {
            self.write_text(" else ");
            self.format_brace_group(body, Some(command.span.end.offset))?;
        }
        Ok(())
    }

    pub(super) fn format_for(&mut self, command: &ForCommand) -> Result<()> {
        self.write_text("for ");
        for (index, target) in command.targets.iter().enumerate() {
            if index > 0 {
                self.write_space();
            }
            self.write_word(&target.word);
        }

        match command.syntax {
            ForSyntax::InDoDone { in_span, .. }
            | ForSyntax::InDirect { in_span }
            | ForSyntax::InBrace { in_span, .. } => {
                self.write_for_in_words(command.words.as_deref(), in_span);
            }
            ForSyntax::ParenDoDone { .. }
            | ForSyntax::ParenDirect { .. }
            | ForSyntax::ParenBrace { .. } => {
                self.write_parenthesized_word_list(command.words.as_deref());
            }
        }

        match command.syntax {
            ForSyntax::InDoDone { done_span, .. } | ForSyntax::ParenDoDone { done_span, .. } => {
                self.format_done_body(&command.body, command.span, Some(done_span))?;
            }
            ForSyntax::InDirect { .. } | ForSyntax::ParenDirect { .. } => {
                self.write_space();
                self.format_inline_stmts(&command.body)?;
            }
            ForSyntax::InBrace { .. } | ForSyntax::ParenBrace { .. } => {
                self.write_text("; ");
                self.format_brace_group(&command.body, Some(command.span.end.offset))?;
            }
        }
        Ok(())
    }

    pub(super) fn write_for_in_words(&mut self, words: Option<&[Word]>, in_span: Option<Span>) {
        if let Some(words) = words {
            self.write_text(" in");
            self.write_word_list_preserving_breaks_after(
                words,
                in_span.map(|span| span.end.offset),
            );
        }
    }

    pub(super) fn write_parenthesized_word_list(&mut self, words: Option<&[Word]>) {
        self.write_text(" (");
        if let Some(words) = words {
            self.write_space_separated_words(words);
        }
        self.write_text(")");
    }

    pub(super) fn write_space_separated_words(&mut self, words: &[Word]) {
        for (index, word) in words.iter().enumerate() {
            if index > 0 {
                self.write_space();
            }
            self.write_word(word);
        }
    }

    pub(super) fn format_repeat(&mut self, command: &RepeatCommand) -> Result<()> {
        self.write_text("repeat ");
        self.write_word(&command.count);
        match command.syntax {
            RepeatSyntax::DoDone { done_span, .. } => {
                self.format_done_body(&command.body, command.span, Some(done_span))?;
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

    pub(super) fn format_foreach(&mut self, command: &ForeachCommand) -> Result<()> {
        self.write_text("foreach ");
        self.write_text(command.variable.as_ref());
        match command.syntax {
            ForeachSyntax::ParenBrace { .. } => {
                self.write_parenthesized_word_list(Some(&command.words));
                self.write_space();
                self.format_brace_group(&command.body, Some(command.span.end.offset))?;
            }
            ForeachSyntax::InDoDone { done_span, .. } => {
                self.write_for_in_words(Some(&command.words), None);
                self.format_done_body(&command.body, command.span, Some(done_span))?;
            }
        }
        Ok(())
    }

    pub(super) fn format_select(&mut self, command: &SelectCommand) -> Result<()> {
        self.write_text("select ");
        self.write_text(command.variable.as_ref());
        self.write_for_in_words(Some(&command.words), None);
        self.format_done_body(&command.body, command.span, None)?;
        Ok(())
    }

    pub(super) fn format_while(&mut self, command: &WhileCommand) -> Result<()> {
        self.format_loop("while", &command.condition, &command.body, command.span)
    }

    pub(super) fn format_until(&mut self, command: &UntilCommand) -> Result<()> {
        self.format_loop("until", &command.condition, &command.body, command.span)
    }

    pub(super) fn format_loop(
        &mut self,
        keyword: &'static str,
        condition: &StmtSeq,
        body: &StmtSeq,
        span: Span,
    ) -> Result<()> {
        let close_span = command_done_close_span(self.source(), self.source_map(), span, None);
        if loop_condition_starts_after_keyword(condition, span)
            || loop_condition_has_multiple_commands(condition)
        {
            self.write_text(keyword);
            self.newline();
            let condition_upper_bound = branch_open_keyword_start(body, self.source(), "do");
            self.with_indent(|formatter| {
                formatter.format_stmt_sequence(condition, condition_upper_bound)
            })?;
            self.newline();
            return self.format_split_do_done_body(body, span, close_span, "done");
        }

        self.write_text(keyword);
        self.write_space();
        self.format_inline_stmts(condition)?;
        self.format_do_done_body(body, span, close_span, "done")
    }

    pub(super) fn format_case(&mut self, command: &CaseCommand) -> Result<()> {
        if !self.options().compact_layout()
            && case_command_was_inline_in_source(command, self.source())
            && self.can_format_case_inline(command)
        {
            return self.format_inline_case(command);
        }

        let case_facts = self.facts().case_command(command);
        let esac_span = case_facts.esac_span();
        let case_body_fallback = case_facts.body_fallback_upper_bound();
        let case_has_blank_line_after_in = case_facts.has_blank_line_after_in();
        let case_has_blank_line_before_esac = case_facts.has_blank_line_before_esac();
        let case_suffix_comments = case_facts.suffix_comments_before_esac().to_vec();
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
                if header_item_count == 0 && index == 0 && case_has_blank_line_after_in {
                    self.newline();
                }
                if index > 0 && self.facts().case_item(item).has_blank_line_before() {
                    self.newline();
                }
                self.format_case_item(item, case_item_body_upper_bound(item, case_body_fallback))?;
            }
            if case_suffix_comments.is_empty() {
                if case_close_shares_line_with_last_item(command, esac_span, self.source()) {
                    self.write_space();
                } else {
                    if case_has_blank_line_before_esac {
                        self.newline();
                    }
                    self.newline();
                }
            } else {
                self.emit_case_suffix_comments_before_esac(
                    command,
                    &case_suffix_comments,
                    esac_span,
                );
            }
            self.write_text("esac");
            self.write_close_suffix_after_span(esac_span);
        }
        Ok(())
    }

    pub(super) fn format_case_items_on_header_line(
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
            if !self.facts().case_item(item).prefix_comments().is_empty() {
                break;
            }
            self.write_space();
            self.format_case_item(item, upper_bound)?;
            item_count += 1;
        }
        Ok(item_count)
    }

    pub(super) fn write_case_open_suffix(&mut self, command: &CaseCommand) {
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

    pub(super) fn format_case_item(
        &mut self,
        item: &CaseItem,
        upper_bound: Option<usize>,
    ) -> Result<()> {
        let base_indent =
            usize::from(!self.options().compact_layout() && self.options().switch_case_indent());
        let first_pattern_start = item
            .patterns
            .first()
            .map(|pattern| pattern.span.start.offset);
        let item_facts = self.facts().case_item(item);
        let prefix_comments = item_facts.prefix_comments().to_vec();
        let pattern_suffix_comment = item_facts.pattern_suffix_comment();
        let has_blank_line_after_pattern = item_facts.has_blank_line_after_pattern();
        let has_blank_line_before_terminator = item_facts.has_blank_line_before_terminator();
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
                if self
                    .facts()
                    .contains_newline_between(previous.span.end.offset, word.span.start.offset)
                {
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
        if let Some(comment) = &pattern_suffix_comment {
            let current_code_column = self.column().saturating_sub(self.line_indent_column());
            let mut padding = trailing_comment_padding(
                self.source(),
                self.source_map(),
                comment,
                current_code_column,
                self.line_indent_column(),
            );
            if trailing_comment_alignment_column(
                self.source(),
                self.source_map(),
                comment,
                self.line_indent_column(),
            )
            .is_some()
            {
                padding += 1;
            }
            self.write_spaces(padding);
            self.write_comment(comment);
        }

        if item.body.is_empty() {
            let body_has_comments = self
                .facts()
                .sequence(&item.body, upper_bound)
                .has_comments();
            let comments = self.empty_case_item_body_comments(item);
            if pattern_suffix_comment.is_some() && !self.options().compact_layout() {
                self.newline();
                self.write_case_prefix(base_indent + 1);
                self.write_case_terminator(item);
                return Ok(());
            }
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
            let pattern_body_terminator_was_inline =
                case_item_pattern_body_terminator_was_inline_in_source(item, self.source());
            let item_was_inline_in_source = self.facts().case_item_was_inline_in_source(item)
                || pattern_body_terminator_was_inline;
            if base_indent == 0
                && item.body.len() == 1
                && case_item_single_body_stmt_can_inline(
                    item,
                    self.source(),
                    self.source_map(),
                    pattern_body_terminator_was_inline,
                )
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
            if (case_item_pattern_close_paren_on_own_line(item, self.source(), self.source_map())
                && !case_item_close_paren_shares_line_with_body(
                    item,
                    self.source(),
                    self.source_map(),
                ))
                || has_blank_line_after_pattern
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
            if has_blank_line_before_terminator {
                self.newline();
            }
            self.newline();
            self.write_case_prefix(base_indent + 1);
            self.write_case_terminator(item);
        }
        Ok(())
    }

    pub(super) fn write_case_terminator(&mut self, item: &CaseItem) {
        self.write_text(case_terminator(item.terminator));
        if let Some(comment) = self.facts().case_item(item).terminator_suffix_comment() {
            self.write_comment_with_padding(&comment, trailing_comment_padding);
        }
    }

    pub(super) fn empty_case_item_body_comments(&self, item: &CaseItem) -> Vec<String> {
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

    pub(super) fn emit_case_suffix_comments_before_esac(
        &mut self,
        command: &CaseCommand,
        comments: &[BranchPrefixComment],
        esac_span: Option<Span>,
    ) {
        let Some(mut previous_line) = command
            .cases
            .last()
            .and_then(|item| self.facts().case_item(item).suffix_comment_start_line())
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

    pub(super) fn emit_case_item_prefix_comments(
        &mut self,
        comments: &[SourceComment<'_>],
        first_pattern: &Pattern,
        base_indent: usize,
    ) {
        let mut body_indent_context = false;
        let mut disabled_case_pattern_context = false;
        for (index, comment) in comments.iter().enumerate() {
            let uses_body_indent = case_prefix_comment_uses_body_indent(
                self.source(),
                self.source_map(),
                comment,
                first_pattern.span.start.offset,
                disabled_case_pattern_context,
                body_indent_context,
            );
            let extra_indent = base_indent + usize::from(uses_body_indent);
            self.with_extra_prefix_indent(extra_indent, |formatter| {
                formatter.write_comment(comment);
            });
            if uses_body_indent {
                body_indent_context = true;
            }
            if comment_looks_like_disabled_case_pattern(comment) {
                disabled_case_pattern_context = true;
            }
            let target_line = comments
                .get(index + 1)
                .map(SourceComment::line)
                .unwrap_or(first_pattern.span.start.line);
            self.write_line_breaks(line_gap_break_count(comment.line(), target_line));
        }
    }

    pub(super) fn with_extra_prefix_indent<T>(
        &mut self,
        levels: usize,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        self.writer.push_indent(levels);
        let result = f(self);
        self.writer.pop_indent(levels);
        result
    }

    pub(super) fn with_pipeline_continuation_indent<T>(
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

    pub(super) fn with_group_body_leading_filter<T>(
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

    pub(super) fn format_brace_group(
        &mut self,
        commands: &StmtSeq,
        upper_bound: Option<usize>,
    ) -> Result<()> {
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

    pub(super) fn format_subshell(
        &mut self,
        commands: &StmtSeq,
        upper_bound: Option<usize>,
    ) -> Result<()> {
        let sequence_facts = self.facts().sequence(commands, upper_bound);
        let should_inline = sequence_facts.group_open_suffix_span().is_none()
            && ((self.group_has_inline_source_shape(commands, '(')
                && self.can_inline_group(commands, '('))
                || self.can_inline_source_line_subshell(commands, upper_bound));
        if should_inline {
            self.write_text("(");
            if stmt_sequence_renders_with_subshell_open(commands) {
                self.write_space();
            }
            self.format_inline_stmts(commands)?;
            self.write_text(")");
            return Ok(());
        }
        if self.can_format_multiline_subshell_inline(commands, upper_bound) {
            self.write_text("(");
            if stmt_sequence_renders_with_subshell_open(commands) {
                self.write_space();
            }
            self.format_stmt_sequence(commands, upper_bound)?;
            self.write_text(")");
            return Ok(());
        }
        self.format_group_with_upper_bound("(", ")", '(', commands, false, upper_bound)
    }

    pub(super) fn format_arithmetic(&mut self, command: &ArithmeticCommand) -> Result<()> {
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
        let mut formatted = format_arithmetic_command_source(rendered);
        if arithmetic_command_is_followed_by_inline_branch_keyword(command.span, self.source()) {
            trim_final_line_ending(&mut formatted);
        }
        self.write_text(&formatted);
        Ok(())
    }

    pub(super) fn format_arithmetic_for(&mut self, command: &ArithmeticForCommand) -> Result<()> {
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
        self.format_done_body(&command.body, command.span, None)
    }

    pub(super) fn format_time(&mut self, command: &TimeCommand) -> Result<()> {
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

    pub(super) fn write_time_inner_trailing_comment(&mut self, stmt: &Stmt) {
        if !time_inner_stmt_needs_trailing_comment(stmt) {
            return;
        }
        let Some(comment) = self
            .facts()
            .close_suffix_comment_after_span(stmt_format_span(stmt))
        else {
            return;
        };
        self.emit_trailing_comments_for_stmt(&[comment]);
    }

    pub(super) fn format_conditional(&mut self, command: &ConditionalCommand) -> Result<()> {
        self.write_text("[[ ");
        self.format_conditional_expr(&command.expression)?;
        let tight_close = self.conditional_needs_tight_close(&command.expression);
        self.write_text(if tight_close { "]]" } else { " ]]" });
        Ok(())
    }

    pub(super) fn format_coproc(&mut self, command: &CoprocCommand) -> Result<()> {
        self.write_text("coproc");
        if command.name.as_str() != "COPROC" || command.name_span.is_some() {
            self.write_space();
            self.write_text(command.name.as_str());
        }
        self.write_space();
        self.format_stmt(&command.body)
    }

    pub(super) fn format_always(&mut self, command: &AlwaysCommand) -> Result<()> {
        self.format_brace_group(&command.body, Some(command.span.end.offset))?;
        self.write_text(" always ");
        self.format_brace_group(&command.always_body, Some(command.span.end.offset))
    }

    pub(super) fn format_function(&mut self, function: &FunctionDef) -> Result<()> {
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

    pub(super) fn format_anonymous_function(
        &mut self,
        function: &AnonymousFunctionCommand,
    ) -> Result<()> {
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

    pub(super) fn format_named_function_header(&mut self, function: &FunctionDef) {
        if function.header.entries.len() == 1
            && let Some(name) = function.header.entries[0].static_name.as_ref()
        {
            let mut rendered_entry = self.take_scratch_buffer();
            self.render_word_with_facts_to_buffer(
                &function.header.entries[0].word,
                &mut rendered_entry,
            );
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

    pub(super) fn format_function_body(&mut self, body: &Stmt, upper_bound: usize) -> Result<()> {
        self.format_function_body_with_header_comment(body, upper_bound, None)
    }

    pub(super) fn format_function_body_with_header_comment(
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

    pub(super) fn format_function_brace_group_with_header_comment(
        &mut self,
        commands: &StmtSeq,
        upper_bound: usize,
        header_comment: &str,
    ) -> Result<()> {
        self.write_text("{");
        let padding =
            self.function_header_comment_padding(commands, Some(upper_bound), self.column());
        self.write_spaces(padding);
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

    pub(super) fn function_header_comment_padding(
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
        let body_indent_column = self.indent_column_for_level(self.indent_level() + 1);
        body_indent_column
            .saturating_add(target_code_column)
            .saturating_sub(header_column)
            .max(1)
    }

    pub(super) fn first_body_inline_comment_target_column(
        &self,
        commands: &StmtSeq,
    ) -> Option<usize> {
        let first = commands.first()?;
        let source = self.source();
        let start = stmt_span(first).start.offset.min(source.len());
        let (line_start, line_end) = self.source_map().line_bounds_for_offset(start)?;
        let width = inline_comment_code_width(source, line_start, line_end, None)?;
        Some(width + 1)
    }

    pub(super) fn function_header_trailing_comment(
        &self,
        function: &FunctionDef,
    ) -> Option<(Span, String)> {
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

    pub(super) fn then_separator_for_condition(&self, commands: &StmtSeq) -> &'static str {
        if self.inline_condition_ends_with_case(commands) {
            " then"
        } else {
            "; then"
        }
    }

    pub(super) fn inline_condition_ends_with_case(&self, commands: &StmtSeq) -> bool {
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

    pub(super) fn format_body_with_upper_bound_and_open_blank(
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

    pub(super) fn format_body_with_upper_bound_open_blank_and_leading_filter(
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

    pub(super) fn finish_block(&mut self, close: &'static str) {
        if self.options().compact_layout() {
            self.write_text("; ");
            self.write_text(close);
        } else {
            self.newline();
            self.write_text(close);
        }
    }

    pub(super) fn finish_block_with_close_suffix(
        &mut self,
        close: &'static str,
        close_span: Option<Span>,
    ) {
        self.finish_block(close);
        self.write_close_suffix_after_span(close_span);
    }

    pub(super) fn format_group_with_upper_bound(
        &mut self,
        open: &'static str,
        close: &'static str,
        _open_char: char,
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
        let sequence_facts = self.facts().sequence(commands, upper_bound);
        let open_suffix_span = sequence_facts.group_open_suffix_span();
        let open_end_offset = sequence_facts.open_end_offset();
        let preserve_open_blank = sequence_facts.has_blank_line_after_open();
        let preserve_close_blank = sequence_facts.has_blank_line_before_close();
        if let Some(span) = open_suffix_span {
            self.write_suffix_comment_after_span(span, true);
        }

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

    pub(super) fn format_conditional_expr(&mut self, expression: &ConditionalExpr) -> Result<()> {
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

    pub(super) fn format_conditional_binary(
        &mut self,
        expression: &ConditionalBinaryExpr,
    ) -> Result<()> {
        self.format_conditional_expr(&expression.left)?;
        self.write_space();
        self.write_text(expression.op.as_str());
        if conditional_binary_has_explicit_rhs_break(expression, self.source_map()) {
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

    pub(super) fn write_conditional_regex_rhs(
        &mut self,
        expression: &ConditionalExpr,
    ) -> Result<()> {
        let raw = expression.span().slice(self.source());
        if raw.contains('\n') {
            self.format_conditional_expr(expression)
        } else {
            let raw = trim_unescaped_trailing_whitespace(raw.trim_start_matches([' ', '\t', '\r']));
            self.write_text(raw);
            Ok(())
        }
    }

    pub(super) fn format_conditional_unary(
        &mut self,
        expression: &ConditionalUnaryExpr,
    ) -> Result<()> {
        self.write_text(expression.op.as_str());
        self.write_space();
        self.format_conditional_expr(&expression.expr)
    }

    pub(super) fn format_conditional_paren(
        &mut self,
        expression: &ConditionalParenExpr,
    ) -> Result<()> {
        self.write_text("(");
        self.format_conditional_expr(&expression.expr)?;
        self.write_text(")");
        Ok(())
    }

    pub(super) fn conditional_needs_tight_close(&mut self, expression: &ConditionalExpr) -> bool {
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

    pub(super) fn conditional_word_needs_tight_close(&mut self, word: &Word) -> bool {
        let mut rendered = self.take_scratch_buffer();
        self.render_word_with_facts_to_buffer(word, &mut rendered);
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

    pub(super) fn write_case_prefix(&mut self, levels: usize) {
        if levels == 0 {
            return;
        }
        self.write_indent_units(levels);
    }
}
