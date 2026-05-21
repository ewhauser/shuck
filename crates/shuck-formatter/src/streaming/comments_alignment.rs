use super::*;

impl<'source, 'facts, S> ShellRenderer<'source, 'facts, S>
where
    S: StreamSink,
{
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

    pub(super) fn emit_trailing_comments_for_stmt(&mut self, comments: &[SourceComment<'source>]) {
        for comment in comments {
            self.write_trailing_comment(comment, false);
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
        let Some(plan) =
            close_span.and_then(|span| self.facts().close_suffix_comment_plan_after_span(span))
        else {
            return;
        };
        self.write_inline_comment_plan(plan, false);
    }

    pub(super) fn write_inline_comment_plan(
        &mut self,
        plan: crate::facts::InlineCommentPlan<'source>,
        nudge_aligned: bool,
    ) {
        let current_code_column = self.column().saturating_sub(self.line_indent_column());
        let mut padding = plan.padding(
            self.source_map(),
            current_code_column,
            self.line_indent_column(),
        );
        if nudge_aligned && plan.has_alignment(self.source_map(), self.line_indent_column()) {
            padding += 1;
        }
        self.write_spaces(padding);
        self.write_comment(&plan.comment());
    }

    pub(super) fn write_trailing_comment(
        &mut self,
        comment: &SourceComment<'source>,
        nudge_aligned: bool,
    ) {
        let plan = self.facts().trailing_comment_plan(*comment);
        self.write_inline_comment_plan(plan, nudge_aligned);
    }

    pub(super) fn write_suffix_comment_after_span(&mut self, span: Span, nudge_aligned: bool) {
        let Some(plan) = self.facts().suffix_comment_plan_for_span(span) else {
            self.write_space();
            self.write_text(span.slice(self.source()).trim_start());
            return;
        };
        self.write_inline_comment_plan(plan, nudge_aligned);
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CommentAlignmentFacts {
    source_indent_column: usize,
    trailing_target_column: Option<usize>,
    trailing_indent_adjust: usize,
    close_suffix_target_column: Option<usize>,
}

impl CommentAlignmentFacts {
    pub(crate) fn new(
        source: &str,
        source_map: &SourceMap<'_>,
        comment: &SourceComment<'_>,
    ) -> Self {
        let source_indent_column = source_map
            .line_indent_before_offset(comment.span().start.offset)
            .map_or(0, |indent| indent.chars().count());
        Self {
            source_indent_column,
            trailing_target_column: trailing_comment_alignment_column(
                source,
                source_map,
                comment,
                source_indent_column,
            ),
            trailing_indent_adjust: trailing_comment_tab_indent_adjust(source, source_map, comment),
            close_suffix_target_column: aligned_close_suffix_comment_target_column(
                source,
                source_map,
                comment,
                source_indent_column,
            ),
        }
    }

    pub(crate) fn trailing_padding(
        &self,
        source: &str,
        source_map: &SourceMap<'_>,
        comment: &SourceComment<'_>,
        current_code_column: usize,
        current_indent_column: usize,
    ) -> usize {
        let Some(target_column) =
            self.trailing_target_column(source, source_map, comment, current_indent_column)
        else {
            return 1;
        };
        target_column
            .saturating_sub(current_code_column.saturating_add(self.trailing_indent_adjust))
            .max(1)
    }

    pub(crate) fn has_trailing_alignment(
        &self,
        source: &str,
        source_map: &SourceMap<'_>,
        comment: &SourceComment<'_>,
        current_indent_column: usize,
    ) -> bool {
        self.trailing_target_column(source, source_map, comment, current_indent_column)
            .is_some()
    }

    pub(crate) fn close_suffix_padding(
        &self,
        source: &str,
        source_map: &SourceMap<'_>,
        comment: &SourceComment<'_>,
        current_code_column: usize,
        current_indent_column: usize,
    ) -> usize {
        if let Some(target_column) =
            self.close_suffix_target_column(source, source_map, comment, current_indent_column)
        {
            return target_column
                .saturating_sub(current_indent_column + current_code_column)
                .max(1);
        }
        self.trailing_padding(
            source,
            source_map,
            comment,
            current_code_column,
            current_indent_column,
        )
    }

    fn trailing_target_column(
        &self,
        source: &str,
        source_map: &SourceMap<'_>,
        comment: &SourceComment<'_>,
        current_indent_column: usize,
    ) -> Option<usize> {
        if current_indent_column == self.source_indent_column {
            return self.trailing_target_column;
        }
        trailing_comment_alignment_column(source, source_map, comment, current_indent_column)
    }

    fn close_suffix_target_column(
        &self,
        source: &str,
        source_map: &SourceMap<'_>,
        comment: &SourceComment<'_>,
        current_indent_column: usize,
    ) -> Option<usize> {
        if current_indent_column == self.source_indent_column {
            return self.close_suffix_target_column;
        }
        aligned_close_suffix_comment_target_column(
            source,
            source_map,
            comment,
            current_indent_column,
        )
    }
}

fn aligned_close_suffix_comment_target_column(
    source: &str,
    source_map: &SourceMap<'_>,
    comment: &SourceComment<'_>,
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
    Some(
        entries
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
            + 1,
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

pub(super) fn trailing_comment_alignment_column(
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

pub(super) fn inline_comment_code_width(
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
