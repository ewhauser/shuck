use super::*;

#[derive(Debug, Clone, Copy)]
pub(super) enum HeredocTailTextMode {
    Rendered,
    Assignment,
}

pub(super) fn assignment_contains_command_heredoc(assignment: &Assignment) -> bool {
    match &assignment.value {
        AssignmentValue::Scalar(word) => word_contains_command_heredoc(word),
        AssignmentValue::Compound(array) => array
            .elements
            .iter()
            .any(|element| word_contains_command_heredoc(array_elem_parts(element).1)),
    }
}

pub(super) fn compound_assignment_is_single_case_command_substitution(
    assignment: &Assignment,
) -> bool {
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

pub(super) fn word_contains_command_heredoc(word: &Word) -> bool {
    word.parts
        .iter()
        .any(|part| word_part_contains_command_heredoc(&part.kind))
}

pub(super) fn heredoc_body_contains_command_substitution(body: &HeredocBody) -> bool {
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

pub(super) fn normalize_escaped_multiline_word_command_substitution_indent(
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

pub(super) fn normalize_rendered_leading_list_operator_continuations(
    rendered: &str,
) -> Option<String> {
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

pub(super) fn normalize_scalar_assignment_unquoted_continuations(
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

pub(super) fn assignment_has_quoted_backslash_continuation_literal(
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

pub(super) fn assignment_source_has_command_substitution(
    assignment: &Assignment,
    source: &str,
) -> bool {
    let raw = assignment.span.slice(source);
    raw.contains("$(") || raw.contains('`') || raw.contains("<(") || raw.contains(">(")
}

pub(super) fn assignment_source_has_leading_pipe_continuation(
    assignment: &Assignment,
    source: &str,
) -> bool {
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

pub(super) fn assignment_value_is_quoted_formattable_command_substitution_only(
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

pub(super) fn assignment_value_is_quoted_command_substitution_only(
    assignment: &Assignment,
) -> bool {
    match &assignment.value {
        AssignmentValue::Scalar(word) => word_is_quoted_command_substitution_only(word),
        AssignmentValue::Compound(_) => false,
    }
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

pub(super) fn pipeline_operator_breaks(
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

pub(super) fn interstitial_comment_end(
    stmt: &Stmt,
    operator_end: usize,
    source: &str,
    source_map: &SourceMap<'_>,
) -> usize {
    stmt_start_after_operator(stmt, operator_end, source, source_map)
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

pub(super) fn normalize_literal_assignment_command_substitution_pipelines(
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

pub(super) fn conditional_binary_has_explicit_rhs_break(
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

pub(super) fn conditional_expr_contains_command_substitution(expression: &ConditionalExpr) -> bool {
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

pub(super) fn word_contains_command_substitution(word: &Word) -> bool {
    word_contains_substitution(word, WordSubstitutionKind::Command)
}

pub(super) fn word_contains_process_substitution(word: &Word) -> bool {
    word_contains_substitution(word, WordSubstitutionKind::Process)
}

pub(super) fn word_source_has_shell_substitution(word: &Word, source: &str) -> bool {
    let raw = word.span.slice(source);
    rendered_text_has_shell_substitution(raw)
}

pub(super) fn rendered_text_has_shell_substitution(text: &str) -> bool {
    text.contains("$(") || text.contains('`') || text.contains("<(") || text.contains(">(")
}

pub(super) fn rendered_text_starts_with_block_command_substitution(text: &str) -> bool {
    text.lines()
        .next()
        .is_some_and(|line| line.trim_end_matches([' ', '\t', '\r']).ends_with("$("))
}

pub(super) fn rendered_text_starts_like_assignment_with_substitution(text: &str) -> bool {
    let first_line = text.lines().next().unwrap_or(text);
    let substitution_start = ["$(", "`", "<(", ">("]
        .iter()
        .filter_map(|marker| first_line.find(marker))
        .min()
        .unwrap_or(first_line.len());
    first_line[..substitution_start].contains('=')
}

pub(super) fn rendered_text_has_leading_list_operator_line(text: &str) -> bool {
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

impl<'source, 'facts, S> ShellRenderer<'source, 'facts, S>
where
    S: StreamSink,
{
    pub(super) fn write_rendered_shell_text(&mut self, text: &str) {
        if text.contains('\n') {
            if self.line_start() {
                self.write_indent();
            }
            self.write_verbatim(text);
        } else {
            self.write_text(text);
        }
    }

    pub(super) fn write_text_preserving_current_line_indent(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        let base_indent_column = if self.line_start() {
            self.indent_column_for_level(self.indent_level())
        } else {
            self.line_indent_column()
        };
        let mut active_heredoc: Option<RenderedHeredocTail> = None;
        let mut remaining = text;
        while !remaining.is_empty() {
            let (line, next, had_newline) = split_first_line(remaining);

            if let Some(heredoc) = active_heredoc.as_ref() {
                if heredoc.strip_tabs {
                    if self.line_start() && !line.is_empty() {
                        self.write_indent_to_column(base_indent_column);
                    }
                    self.push_output_str(line);
                    self.writer.set_line_start(false);
                } else {
                    self.write_verbatim(heredoc.body_line(line));
                }
                if heredoc.closes(line) {
                    active_heredoc = None;
                }
            } else {
                if self.line_start() && !line.is_empty() {
                    self.write_indent_to_column(base_indent_column);
                }
                self.push_output_str(line);
                self.writer.set_line_start(false);
                active_heredoc = rendered_heredoc_tail_start(line);
            }

            if had_newline {
                self.push_output_str(self.line_ending());
                self.writer.set_line_start(true);
            }
            remaining = next;
        }
    }

    pub(super) fn write_command_substitution_assignment_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        let base_indent_column = if self.line_start() {
            self.indent_column_for_level(self.indent_level())
        } else {
            self.line_indent_column()
        };
        let starts_with_block_command_substitution = text
            .lines()
            .next()
            .is_some_and(|line| line.trim_end_matches([' ', '\t', '\r']).ends_with("$("));
        let strip_context_indent = !starts_with_block_command_substitution;
        let indent_unit = self.options.indent_unit_columns();
        let inline_pipeline_indent_column = base_indent_column + indent_unit;
        let mut next_pipeline_indent_column = None;
        let mut active_shell_pipeline_indent_column: Option<usize> = None;
        let mut active_shell_line_was_pipeline_stage = false;
        let mut next_block_line_is_pipeline_stage = false;
        let mut next_block_line_aligns_with_command_continuation = false;
        let mut command_continuation_active = false;
        let mut pipeline_quote_state = RawLineQuoteState::default();
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
            let pipeline_stage_indent = self.line_start()
                && !remaining.starts_with('\n')
                && pipeline_indent_column.is_some()
                && !closes_block_command_substitution
                && !remaining
                    .trim_start_matches([' ', '\t', '\r'])
                    .starts_with('\n');
            let add_context_indent = self.line_start()
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

            let (line, next, had_newline) = split_first_line_including_newline(remaining);
            let line = if pipeline_stage_indent {
                line.trim_start_matches([' ', '\t'])
            } else if add_context_indent && strip_context_indent {
                strip_assignment_context_indent(line, base_indent_column, self.options())
            } else {
                line
            };
            if had_newline {
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
                    && line_without_continuation_backslash(line.trim_end_matches('\n')).is_some();
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
                self.writer.set_line_start(true);
                remaining = next;
            } else {
                self.push_output_str(line);
                self.writer.set_line_start(false);
                break;
            }
        }
    }

    pub(super) fn write_shell_text_with_heredoc_tails(
        &mut self,
        text: &str,
        assignment_context: bool,
    ) {
        if assignment_context && !rendered_text_starts_with_block_command_substitution(text) {
            let base_indent_column = if self.line_start() {
                self.indent_column_for_level(self.indent_level())
            } else if self.line_indent_column() > 0 {
                self.line_indent_column()
            } else {
                self.column()
            };
            if self.line_start() {
                self.write_indent_to_column(base_indent_column);
            }
            self.write_shell_text_preserving_heredoc_tails(text, HeredocTailTextMode::Assignment);
            return;
        }
        self.write_shell_text_preserving_heredoc_tails(text, HeredocTailTextMode::Rendered);
    }

    pub(super) fn write_shell_text_preserving_heredoc_tails(
        &mut self,
        text: &str,
        mode: HeredocTailTextMode,
    ) {
        let mut active_heredoc: Option<RenderedHeredocTail> = None;
        let mut rest = text;

        while !rest.is_empty() {
            let (line, next, had_newline) = split_first_line(rest);

            if let Some(heredoc) = active_heredoc.as_ref() {
                self.write_verbatim(heredoc.body_line(line));
                if heredoc.closes(line) {
                    active_heredoc = None;
                }
            } else {
                let heredoc = rendered_heredoc_tail_start(line);
                let normalized = heredoc
                    .is_some()
                    .then(|| normalize_rendered_heredoc_start_spacing(line))
                    .flatten();
                let line = if self.options().space_redirects() {
                    line
                } else {
                    normalized.as_deref().unwrap_or(line)
                };
                match mode {
                    HeredocTailTextMode::Rendered => self.write_text(line),
                    HeredocTailTextMode::Assignment => self.write_verbatim(line),
                }
                active_heredoc = heredoc;
            }

            if had_newline {
                self.push_output_str(self.line_ending());
                self.writer.set_line_start(true);
            }
            rest = next;
        }
    }

    pub(super) fn write_word(&mut self, word: &Word) {
        let mut scratch = self.take_scratch_buffer();
        self.render_word_to_buffer(word, &mut scratch);
        if rendered_shell_text_has_heredoc_tail(&scratch)
            && (word_contains_command_heredoc(word)
                || word_source_has_shell_substitution(word, self.source())
                || rendered_text_has_shell_substitution(&scratch))
        {
            self.write_shell_text_with_heredoc_tails(&scratch, true);
        } else if scratch.contains('\n')
            && (word_is_quoted_formattable_command_substitution_only_with_facts(word, self.facts())
                || word_contains_process_substitution(word))
        {
            self.write_text_preserving_current_line_indent(&scratch);
        } else if self.facts().word_has_multiline_literal_source(word) {
            if scratch.contains('\n')
                && (word_contains_command_substitution(word)
                    || rendered_text_has_shell_substitution(&scratch))
                && let Some(normalized) =
                    normalize_rendered_leading_list_operator_continuations(&scratch)
            {
                self.write_command_substitution_assignment_text(&normalized);
            } else {
                self.write_rendered_shell_text(&scratch);
            }
        } else if scratch.contains('\n')
            && (word_contains_command_substitution(word)
                || rendered_text_has_shell_substitution(&scratch))
        {
            if rendered_text_has_leading_list_operator_line(&scratch) {
                self.write_command_substitution_assignment_text(&scratch);
            } else {
                self.write_text_preserving_current_line_indent(&scratch);
            }
        } else {
            self.write_text(&scratch);
        }
        self.restore_scratch_buffer(scratch);
    }

    pub(super) fn write_pattern(&mut self, pattern: &Pattern) {
        let mut scratch = self.take_scratch_buffer();
        render_pattern_syntax_to_buf(pattern, self.render_context(), &mut scratch);
        self.write_text(&scratch);
        self.restore_scratch_buffer(scratch);
    }

    pub(super) fn write_case_pattern(&mut self, item: &CaseItem, pattern: &Pattern) {
        let mut scratch = self.take_scratch_buffer();
        render_pattern_syntax_to_buf(pattern, self.render_context(), &mut scratch);
        if case_item_pattern_close_paren_on_own_line(item, self.source(), self.source_map()) {
            trim_trailing_pattern_line_continuation(&mut scratch);
        }
        self.write_text(&scratch);
        self.restore_scratch_buffer(scratch);
    }

    pub(super) fn write_var_ref(&mut self, reference: &VarRef) {
        self.write_rendered(|scratch, source, _| {
            render_var_ref_to_buf(reference, source, scratch);
        });
    }

    pub(super) fn write_assignment(&mut self, assignment: &Assignment) {
        if assignment_has_quoted_backslash_continuation_literal(assignment, self.source()) {
            self.write_rendered_shell_text(assignment.span.slice(self.source()));
            return;
        }
        if let Some(normalized) = normalize_scalar_assignment_unquoted_continuations(
            assignment,
            self.source(),
            self.facts(),
        ) {
            self.write_text(&normalized);
            return;
        }

        let mut scratch = self.take_scratch_buffer();
        render_assignment_to_buf(assignment, self.render_context(), &mut scratch);
        if rendered_shell_text_has_heredoc_tail(&scratch)
            && (assignment_contains_command_heredoc(assignment)
                || assignment_source_has_command_substitution(assignment, self.source())
                || rendered_text_has_shell_substitution(&scratch))
        {
            self.write_shell_text_with_heredoc_tails(&scratch, true);
        } else if scratch.contains('\n')
            && assignment_value_is_quoted_formattable_command_substitution_only(
                assignment,
                self.facts(),
            )
        {
            self.write_text_preserving_current_line_indent(&scratch);
        } else if scratch.contains('\n')
            && assignment_source_has_command_substitution(assignment, self.source())
        {
            if compound_assignment_is_single_case_command_substitution(assignment) {
                self.write_text_preserving_current_line_indent(&scratch);
            } else if self
                .facts()
                .assignment_has_multiline_literal_source(assignment, self.source())
            {
                if assignment_value_is_quoted_command_substitution_only(assignment) {
                    self.write_command_substitution_assignment_text(&scratch);
                } else if assignment_source_has_leading_pipe_continuation(assignment, self.source())
                {
                    self.write_text_preserving_current_line_indent(&scratch);
                } else {
                    let continuation_indent = self
                        .options
                        .indent_prefix(self.indent_level().saturating_add(1));
                    let normalized = normalize_literal_assignment_command_substitution_pipelines(
                        &scratch,
                        &continuation_indent,
                    );
                    self.write_rendered_shell_text(&normalized);
                }
            } else {
                self.write_text_preserving_current_line_indent(&scratch);
            }
        } else if self
            .facts()
            .assignment_has_multiline_literal_source(assignment, self.source())
        {
            self.write_rendered_shell_text(&scratch);
        } else {
            self.write_text(&scratch);
        }
        self.restore_scratch_buffer(scratch);
    }

    pub(super) fn write_assignment_head(&mut self, assignment: &Assignment) {
        self.write_rendered(|scratch, source, _| {
            render_assignment_head_to_buf(assignment, source, scratch);
        });
    }

    pub(super) fn write_rendered_name_text(&mut self, rendered_name: &str) {
        if rendered_shell_text_has_heredoc_tail(rendered_name)
            && rendered_text_has_shell_substitution(rendered_name)
        {
            self.write_shell_text_with_heredoc_tails(
                rendered_name,
                rendered_text_starts_like_assignment_with_substitution(rendered_name),
            );
        } else {
            self.write_text(rendered_name);
        }
    }

    pub(super) fn format_standalone_multiline_compound_assignment(
        &mut self,
        assignment: &shuck_ast::Assignment,
    ) -> Result<()> {
        let source = self.source();
        if compound_assignment_is_single_case_command_substitution(assignment) {
            self.write_assignment(assignment);
            return Ok(());
        }

        if self.format_escaped_multiline_double_quoted_compound_assignment(assignment) {
            return Ok(());
        }

        if self.compound_assignment_should_preserve_multiline_literal_layout(assignment) {
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

    pub(super) fn compound_assignment_source_has_line_continuations(raw: &str) -> bool {
        raw.contains("\\\n") || raw.contains("\\\r\n")
    }

    pub(super) fn compound_assignment_source_has_escaped_multiline_double_quoted_item(
        raw: &str,
    ) -> bool {
        if !Self::compound_assignment_source_has_line_continuations(raw) {
            return false;
        }

        let Some(open) = raw.find('(') else {
            return false;
        };
        let Some(close) = raw.rfind(')') else {
            return false;
        };
        if close <= open {
            return false;
        }

        raw.get(open + 1..close)
            .is_some_and(|body| body.contains("\"\\\n") || body.contains("\"\\\r\n"))
    }

    pub(super) fn compound_assignment_should_preserve_multiline_literal_layout(
        &self,
        assignment: &Assignment,
    ) -> bool {
        let source = self.source();
        if !self
            .facts()
            .assignment_has_multiline_literal_source(assignment, source)
        {
            return false;
        }

        let raw = assignment.span.slice(source);
        !Self::compound_assignment_source_has_line_continuations(raw)
    }

    pub(super) fn format_escaped_multiline_double_quoted_compound_assignment(
        &mut self,
        assignment: &Assignment,
    ) -> bool {
        if !Self::compound_assignment_source_has_escaped_multiline_double_quoted_item(
            assignment.span.slice(self.source()),
        ) {
            return false;
        }

        let AssignmentValue::Compound(array) = &assignment.value else {
            return false;
        };

        self.write_assignment_head(assignment);
        self.write_text("(");
        for (index, element) in array.elements.iter().enumerate() {
            if index > 0 {
                self.newline();
                self.write_indent_units(1);
            }
            self.write_array_element(element, true);
        }
        self.write_text(")");
        true
    }

    pub(super) fn write_word_with_escaped_multiline_substitution_indent(&mut self, word: &Word) {
        let mut scratch = self.take_scratch_buffer();
        render_escaped_multiline_word_syntax_to_buf(word, self.render_context(), &mut scratch);

        let normalized =
            normalize_escaped_multiline_word_command_substitution_indent(&scratch, self.options());
        let rendered = normalized.as_deref().unwrap_or(&scratch);
        if rendered.contains('\n')
            && rendered_text_has_shell_substitution(rendered)
            && let Some(normalized) =
                normalize_rendered_leading_list_operator_continuations(rendered)
        {
            self.write_command_substitution_assignment_text(&normalized);
        } else if rendered.contains('\n') {
            self.write_rendered_shell_text(rendered);
        } else {
            self.write_text(rendered);
        }
        self.restore_scratch_buffer(scratch);
    }

    pub(super) fn write_multiline_compound_literal_assignment(&mut self, assignment: &Assignment) {
        let raw = assignment.span.slice(self.source());
        let Some((head, tail)) = raw.split_once('\n') else {
            self.write_text(raw);
            return;
        };

        self.write_text(head);
        let mut quote = multiline_literal_quote_state_after_line(head, None);
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
}
