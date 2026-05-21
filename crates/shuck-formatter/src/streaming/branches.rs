use super::*;

pub(super) fn if_branch_upper_bound(
    command: &IfCommand,
    branch_index: usize,
    source: &str,
    source_map: &SourceMap<'_>,
    facts: &FormatterFacts<'_>,
) -> usize {
    if let Some((start, end)) = if_next_branch_region(command, branch_index, source, facts) {
        facts
            .branch_prefix_first_comment_offset(start, end)
            .unwrap_or(end)
    } else {
        command_if_close_span(command, source, source_map)
            .start
            .offset
    }
}

fn if_next_branch_region(
    command: &IfCommand,
    branch_index: usize,
    source: &str,
    facts: &FormatterFacts<'_>,
) -> Option<(usize, usize)> {
    if_next_branch_region_with_body_end(command, branch_index, source, |body| {
        branch_body_content_end(body, source, facts)
    })
}

fn branch_body_content_end(body: &StmtSeq, source: &str, facts: &FormatterFacts<'_>) -> usize {
    let mut end = body
        .last()
        .map(|stmt| stmt_span(stmt).end.offset)
        .unwrap_or(body.span.end.offset);
    if let Some(stmt) = body.last() {
        for redirect in &stmt.redirects {
            let Some(heredoc) = redirect.heredoc() else {
                continue;
            };
            let heredoc_end = facts
                .heredoc_closing_marker_bounds(heredoc)
                .map(|(_, line_end)| line_end)
                .unwrap_or(heredoc.body.span.end.offset);
            end = end.max(heredoc_end);
        }
    }
    let end = end.min(source.len());
    trim_trailing_gap_before_offset(source, end)
}

fn comment_looks_like_disabled_if_branch(text: &str) -> bool {
    let body = text
        .strip_prefix('#')
        .unwrap_or(text)
        .trim_start_matches([' ', '\t']);
    ["elif", "else"]
        .iter()
        .any(|keyword| shell_keyword_at(body, 0, body.len(), keyword))
}

pub(super) fn branch_prefix_comments_use_disabled_body_indent(
    comments: &[BranchPrefixComment],
) -> bool {
    let Some(first) = comments.first() else {
        return false;
    };
    comment_looks_like_disabled_if_branch(&first.text)
        && comments
            .iter()
            .skip(1)
            .any(|comment| comment.source_indent > first.source_indent)
}

pub(super) fn unmodeled_branch_background_operator(
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
    let trimmed = between.trim_start_matches([' ', '\t', '\r', '\n']);
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
    let trimmed = text.trim_end_matches([' ', '\t', '\r', '\n']);
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
