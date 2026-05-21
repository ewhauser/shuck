use super::*;

pub(super) fn if_condition_starts_after_keyword(
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

pub(super) fn if_condition_has_explicit_statement_break(
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

pub(super) fn condition_stmt_command_end(stmt: &Stmt) -> usize {
    let mut end = command_format_span(&stmt.command).end.offset;
    if end == 0 {
        end = stmt_span(stmt).end.offset;
    }
    for redirect in &stmt.redirects {
        end = end.max(redirect.span.end.offset);
    }
    end
}

pub(super) fn elif_condition_has_explicit_statement_break(
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

pub(super) fn loop_condition_starts_after_keyword(condition: &StmtSeq, span: Span) -> bool {
    condition
        .first()
        .is_some_and(|stmt| stmt_span(stmt).start.line > span.start.line)
}

pub(super) fn condition_keyword_on_previous_non_empty_line(
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

pub(super) fn raw_grouped_if_condition(
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

pub(super) fn stmt_sequence_renders_with_subshell_open(commands: &StmtSeq) -> bool {
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
