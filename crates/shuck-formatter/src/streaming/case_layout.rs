use super::*;

pub(super) fn case_command_was_inline_in_source(command: &CaseCommand, source: &str) -> bool {
    command.span.slice(source).lines().nth(1).is_none()
}

pub(super) fn case_item_body_terminator_was_inline_in_source(item: &CaseItem) -> bool {
    let [stmt] = item.body.as_slice() else {
        return false;
    };
    item.terminator_span
        .is_some_and(|span| span.start.line == stmt_format_span(stmt).end.line)
}

pub(super) fn case_item_pattern_body_terminator_was_inline_in_source(
    item: &CaseItem,
    source: &str,
) -> bool {
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

pub(super) fn case_item_body_can_share_terminator(item: &CaseItem) -> bool {
    let [stmt] = item.body.as_slice() else {
        return false;
    };
    matches!(
        stmt.command,
        Command::Simple(_) | Command::Builtin(_) | Command::Decl(_)
    ) && stmt.redirects.is_empty()
        && stmt.terminator.is_none()
}

pub(super) fn case_item_single_body_stmt_can_inline(
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

pub(super) fn case_item_body_was_inline_without_terminator(item: &CaseItem) -> bool {
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

pub(super) fn case_close_shares_line_with_last_item(
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

pub(super) fn case_item_started_inline_without_terminator(item: &CaseItem) -> bool {
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

pub(super) fn case_item_pattern_starts_on_case_header(
    command: &CaseCommand,
    item: &CaseItem,
) -> bool {
    item.patterns
        .first()
        .is_some_and(|pattern| pattern.span.start.line == command.span.start.line)
}

pub(super) fn case_item_pattern_close_paren_on_own_line(
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

pub(super) fn case_item_close_paren_shares_line_with_body(
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

pub(super) fn trim_trailing_pattern_line_continuation(rendered: &mut String) {
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

pub(super) fn case_prefix_comment_uses_body_indent(
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

pub(super) fn comment_looks_like_disabled_case_pattern(comment: &SourceComment<'_>) -> bool {
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
