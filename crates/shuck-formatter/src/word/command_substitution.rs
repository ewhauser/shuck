use super::raw_rewrites::*;
use super::*;

pub(super) fn push_raw_word_with_normalized_command_redirect_spacing(
    rendered: &mut String,
    word: &Word,
    raw: &str,
    source: &str,
    options: &ResolvedShellFormatOptions,
) {
    let mut spans = Vec::new();
    collect_raw_command_substitution_spans(word.parts.as_slice(), &mut spans);
    spans.sort_by_key(|span| span.start.offset);
    let mut cursor = word.span.start.offset;
    let word_end = word.span.end.offset.min(source.len());
    let mut wrote_span = false;
    for span in spans {
        let start = span.start.offset;
        let end = span.end.offset;
        if start < cursor || end > word_end || start >= end {
            continue;
        }
        if let Some(prefix) = source.get(cursor..start) {
            rendered.push_str(prefix);
        }
        if let Some(command) = source.get(start..end) {
            push_raw_command_substitution_with_normalized_spacing(
                rendered, command, source, start, options,
            );
            wrote_span = true;
        }
        cursor = end;
    }
    if wrote_span {
        if let Some(suffix) = source.get(cursor..word_end) {
            rendered.push_str(suffix);
        }
    } else {
        rendered.push_str(raw);
    }
}

pub(super) fn push_raw_command_substitution_with_normalized_spacing(
    target: &mut String,
    raw: &str,
    source: &str,
    start_offset: usize,
    options: &ResolvedShellFormatOptions,
) {
    if let Some(normalized) = normalize_raw_backtick_command_substitution(raw) {
        target.push_str(&normalized);
        return;
    }
    if !raw.contains('\n') {
        push_raw_shell_text_with_normalized_redirect_spacing(target, raw);
        return;
    }
    let normalized_pipeline = normalize_raw_pipeline_continuations(raw);
    let raw = normalized_pipeline.as_deref().unwrap_or(raw);
    let normalized_close_continuations =
        normalize_continuations_before_substitution_close_lines(raw);
    let raw = normalized_close_continuations.as_deref().unwrap_or(raw);
    let outer_indent = line_indent_before_source_offset(source, start_offset).unwrap_or("");
    let mut quote = QuoteState::default();
    let raw_lines = raw.split('\n').collect::<Vec<_>>();
    let Some((first, lines)) = raw_lines.split_first() else {
        return;
    };
    target.push_str(first);
    quote.scan_line(first);
    let mut previous_pipeline_indent: Option<String> = None;
    let mut continuation_pipeline_stage_indent: Option<String> = None;
    let mut compound_indents = RawCompoundIndentState::default();
    let outer_shell_indent = normalized_raw_shell_indent(outer_indent, options);
    let mut continuation_indent: Option<String> = line_without_continuation_backslash(first)
        .and_then(|continued| {
            let starts_command_substitution =
                first.trim_start_matches([' ', '\t']).starts_with("$(");
            (starts_command_substitution && !continued.contains(')'))
                .then(|| source_indent_plus_one_unit(&outer_shell_indent, options))
        });
    let mut literal_exit_continuation_indent: Option<String> = None;
    for (line_index, line) in lines.iter().enumerate() {
        let line = *line;
        target.push('\n');
        if quote.in_multiline_literal() {
            let line_continues = line_without_continuation_backslash(line).is_some();
            if let Some(previous_indent) = continuation_indent.as_deref() {
                let stripped = line
                    .strip_prefix(outer_indent)
                    .unwrap_or_else(|| strip_one_indent_unit(line, options));
                let content = stripped.trim_start_matches([' ', '\t']);
                target.push_str(previous_indent);
                target.push_str(content);
            } else {
                target.push_str(line);
            }
            quote.scan_line(line);
            continuation_indent = if line_continues {
                if quote.in_multiline_literal() {
                    continuation_indent.clone()
                } else {
                    continuation_indent
                        .clone()
                        .or_else(|| literal_exit_continuation_indent.take())
                        .or_else(|| Some(source_indent_plus_one_unit("", options)))
                }
            } else {
                if !quote.in_multiline_literal() {
                    literal_exit_continuation_indent = None;
                }
                None
            };
            continue;
        } else {
            let mut line = strip_outer_indent_or_one_unit(line, outer_indent, options).to_string();
            let source_indent_for_compound_shift = line_leading_shell_indent(&line).to_string();
            if let Some(shifted) = compound_indents.shifted_line(&line, options) {
                line = shifted;
            }
            let (indent, content) = raw_line_parts(&line);
            let carried_pipeline_indent = previous_pipeline_indent.clone();
            if let Some(previous_indent) = carried_pipeline_indent.as_deref()
                && !content.trim().is_empty()
                && raw_indent_units(indent, options) < raw_indent_units(previous_indent, options)
            {
                line = format!("{previous_indent}{content}");
            }
            let (indent, content) = raw_line_parts(&line);
            let closes_substitution_wrapper = raw_line_closes_substitution_wrapper(content)
                && raw_block_line_is_outer_substitution_close(lines, line_index);
            if let Some(previous_indent) = continuation_indent.as_deref()
                && !content.trim().is_empty()
                && !content.starts_with('#')
                && !closes_substitution_wrapper
                && normalized_raw_shell_indent(indent, options) != previous_indent
            {
                line = format!("{previous_indent}{content}");
            }
            let (indent, content) = raw_line_parts(&line);
            if let Some(child_indent) =
                compound_indents.child_indent_if_underindented(indent, content, options)
            {
                line = format!("{child_indent}{content}");
            }
            let (indent, content) = raw_line_parts(&line);
            let used_continuation_indent = continuation_indent.is_some();
            let rendered_indent = if closes_substitution_wrapper {
                push_raw_shell_line_with_rendered_indent(target, &line, options, "");
                String::new()
            } else {
                push_raw_shell_line_with_normalized_source_indent(target, &line, options, None);
                rendered_raw_shell_indent_for_line(indent, content, None, options)
            };
            let line_closes_pipeline_stage_compound =
                compound_indents.closes_pipeline_stage(content);
            let line_is_pipeline_continuation_stage = carried_pipeline_indent.is_some();
            let continued_pipeline_stage_indent = continuation_pipeline_stage_indent.clone();
            previous_pipeline_indent = if content.trim().is_empty() {
                None
            } else if content.starts_with('#') {
                carried_pipeline_indent
            } else if line_ends_with_raw_continuation_operator(&line) {
                carried_pipeline_indent.or_else(|| {
                    let indent = line_leading_shell_indent(&line);
                    Some(
                        if content.starts_with('-') || line_closes_pipeline_stage_compound {
                            if raw_line_closes_inline_brace_group_before_pipeline(content) {
                                continued_pipeline_stage_indent.unwrap_or_else(|| {
                                    source_indent_minus_one_unit(indent, options)
                                })
                            } else {
                                indent.to_string()
                            }
                        } else {
                            source_indent_plus_one_unit(indent, options)
                        },
                    )
                })
            } else {
                None
            };
            let line_continues = line_without_continuation_backslash(&line).is_some();
            let line_indent = line_leading_shell_indent(&line).to_string();
            quote.scan_line(&line);
            compound_indents.update_line(
                content,
                &source_indent_for_compound_shift,
                &rendered_indent,
                indent,
                line_is_pipeline_continuation_stage,
                options,
            );
            if line_continues {
                if line_is_pipeline_continuation_stage && !used_continuation_indent {
                    continuation_pipeline_stage_indent = Some(line_indent.clone());
                }
            } else if used_continuation_indent {
                continuation_pipeline_stage_indent = None;
            }
            if quote.in_multiline_literal() && used_continuation_indent {
                literal_exit_continuation_indent = Some(line_indent.clone());
            }
            continuation_indent = if line_continues {
                Some(
                    if quote.in_multiline_literal() || used_continuation_indent {
                        line_indent
                    } else {
                        source_indent_plus_one_unit(&line_indent, options)
                    },
                )
            } else {
                None
            };
            continue;
        }
    }
}

pub(super) fn collect_raw_command_substitution_spans(
    parts: &[shuck_ast::WordPartNode],
    spans: &mut Vec<shuck_ast::Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::CommandSubstitution { .. } => spans.push(part.span),
            WordPart::DoubleQuoted { parts, .. } => {
                collect_raw_command_substitution_spans(parts.as_slice(), spans);
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct RawCommandSubstitutionCommentFallback<'source, 'facts> {
    pub(super) raw: &'source str,
    pub(super) body: &'source shuck_ast::StmtSeq,
    pub(super) source: &'source str,
    pub(super) span_start: usize,
    pub(super) options: &'source ResolvedShellFormatOptions,
    pub(super) facts: Option<&'source FormatterFacts<'facts>>,
}

pub(super) fn push_raw_command_substitution_comment_fallback(
    rendered: &mut String,
    fallback: RawCommandSubstitutionCommentFallback<'_, '_>,
    try_normalized_body: bool,
) {
    let RawCommandSubstitutionCommentFallback {
        raw,
        body,
        source,
        span_start,
        options,
        facts,
    } = fallback;

    if push_inline_raw_command_substitution_as_block(rendered, raw, options) {
        return;
    }
    if command_substitution_source_starts_with_body_line(raw) && !stmt_seq_has_heredoc(facts, body)
    {
        push_raw_block_command_substitution_without_outer_indent(
            rendered, raw, source, span_start, options,
        );
        return;
    }
    if try_normalized_body
        && push_inline_raw_command_substitution_with_normalized_body(rendered, raw, options)
    {
        return;
    }
    push_raw_shell_text_with_normalized_redirect_spacing(rendered, raw);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_command_substitution(
    rendered: &mut String,
    body: &shuck_ast::StmtSeq,
    upper_bound: usize,
    source: &str,
    options: &ResolvedShellFormatOptions,
    layout: CommandSubstitutionLayout,
    inline_continuation_indent_levels: usize,
    raw: Option<&str>,
    _source_map: Option<&SourceMap<'_>>,
    facts: Option<&FormatterFacts<'_>>,
) -> Option<()> {
    let mut nested = String::new();
    format_nested_stmt_sequence_to_buf(
        source,
        body,
        options,
        facts,
        Some(upper_bound),
        &mut nested,
    )?;

    let trimmed = trim_trailing_line_endings(&nested);
    let normalized_backtick_body;
    let trimmed = if raw.is_some_and(|raw| raw.starts_with('`')) && trimmed.contains("\\\\$") {
        normalized_backtick_body = normalize_backtick_body_escaped_dollars(trimmed);
        normalized_backtick_body.as_str()
    } else {
        trimmed
    };
    if trimmed.is_empty() {
        if raw
            .and_then(raw_dollar_command_substitution_body)
            .is_some_and(|body| !body.trim_matches([' ', '\t', '\r', '\n']).is_empty())
        {
            return None;
        }
        rendered.push_str("$()");
        return Some(());
    }
    let normalized_close_continuation = trim_rendered_close_line_continuation(trimmed);
    let trimmed = normalized_close_continuation.as_deref().unwrap_or(trimmed);
    let trailing_escaped_whitespace = raw
        .and_then(raw_command_substitution_trailing_escaped_horizontal_whitespace)
        .or_else(|| {
            source_trailing_escaped_horizontal_whitespace_before_offset(source, upper_bound)
        });

    match layout {
        CommandSubstitutionLayout::Inline | CommandSubstitutionLayout::InlineContinued => {
            rendered.push_str("$(");
            let trimmed = trim_inline_command_substitution_padding(trimmed);
            if let Some(body) =
                restore_trailing_escaped_horizontal_whitespace(trimmed, trailing_escaped_whitespace)
            {
                push_command_substitution_inline_body(
                    rendered,
                    &body,
                    options,
                    inline_continuation_indent_levels,
                );
            } else {
                push_command_substitution_inline_body(
                    rendered,
                    trimmed,
                    options,
                    inline_continuation_indent_levels,
                );
            }
            rendered.push(')');
        }
        CommandSubstitutionLayout::InlineSourceIndented => {
            rendered.push_str("$(");
            push_source_indented_inline_command_substitution(rendered, trimmed, raw?, options);
            rendered.push(')');
        }
        CommandSubstitutionLayout::Block => {
            rendered.push_str("$(\n");
            push_indented_rendered_block(rendered, trimmed, options, 1);
            rendered.push_str("\n)");
        }
    }

    Some(())
}

pub(super) fn format_nested_stmt_sequence_to_buf(
    source: &str,
    body: &StmtSeq,
    options: &ResolvedShellFormatOptions,
    facts: Option<&FormatterFacts<'_>>,
    upper_bound: Option<usize>,
    rendered: &mut String,
) -> Option<()> {
    let owned_facts;
    let facts = match facts {
        Some(facts) => facts,
        None => {
            let file = shuck_ast::File {
                body: body.clone(),
                span: body.span,
            };
            owned_facts = FormatterFacts::build(source, &file, options);
            &owned_facts
        }
    };
    format_stmt_sequence_streaming_to_buf(source, body, options, facts, upper_bound, rendered).ok()
}

pub(super) fn restore_trailing_escaped_horizontal_whitespace(
    body: &str,
    escaped_whitespace: Option<char>,
) -> Option<String> {
    let whitespace = escaped_whitespace?;
    body.ends_with('\\').then(|| {
        let mut restored = body.to_string();
        restored.push(whitespace);
        restored
    })
}

pub(super) fn raw_command_substitution_trailing_escaped_horizontal_whitespace(
    raw: &str,
) -> Option<char> {
    let body = raw_dollar_command_substitution_body(raw)?;
    trailing_escaped_horizontal_whitespace(body)
}

pub(super) fn source_trailing_escaped_horizontal_whitespace_before_offset(
    source: &str,
    upper_bound: usize,
) -> Option<char> {
    let close_offset = upper_bound.checked_sub(1)?;
    if source.as_bytes().get(close_offset) != Some(&b')') {
        return None;
    }
    trailing_escaped_horizontal_whitespace(source.get(..close_offset)?)
}

pub(super) fn trailing_escaped_horizontal_whitespace(body: &str) -> Option<char> {
    let (whitespace_start, whitespace) = body.char_indices().next_back()?;
    if !matches!(whitespace, ' ' | '\t') {
        return None;
    }
    let backslash_count = body.as_bytes()[..whitespace_start]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count();
    (backslash_count % 2 == 1).then_some(whitespace)
}

pub(super) fn trim_rendered_close_line_continuation(rendered: &str) -> Option<String> {
    let trimmed = rendered.trim_end_matches([' ', '\t']);
    if let Some((before_close, close_line)) = trimmed.rsplit_once('\n')
        && close_line.trim_matches([' ', '\t', '\r']) == ")"
    {
        let before_close = before_close.trim_end_matches([' ', '\t']);
        return has_odd_trailing_backslashes(before_close).then(|| {
            before_close[..before_close.len().saturating_sub(1)]
                .trim_end_matches([' ', '\t'])
                .to_string()
        });
    }
    has_odd_trailing_backslashes(trimmed).then(|| {
        trimmed[..trimmed.len().saturating_sub(1)]
            .trim_end_matches([' ', '\t'])
            .to_string()
    })
}

pub(super) fn has_odd_trailing_backslashes(text: &str) -> bool {
    text.as_bytes()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
        % 2
        == 1
}

pub(super) fn commented_command_substitution_can_use_structural_formatter(body: &StmtSeq) -> bool {
    let [stmt] = body.as_slice() else {
        return false;
    };
    !stmt.negated
        && stmt.redirects.is_empty()
        && stmt.terminator.is_none()
        && (matches!(
            &stmt.command,
            Command::Compound(CompoundCommand::Case(_) | CompoundCommand::If(_))
        ) || command_is_pipeline_of_compound_groups(&stmt.command))
}

pub(super) fn command_is_pipeline_of_compound_groups(command: &Command) -> bool {
    let Command::Binary(binary) = command else {
        return false;
    };
    matches!(binary.op, BinaryOp::Pipe | BinaryOp::PipeAll)
        && stmt_is_compound_group_pipeline_operand(&binary.left)
        && stmt_is_compound_group_pipeline_operand(&binary.right)
}

pub(super) fn stmt_is_compound_group_pipeline_operand(stmt: &Stmt) -> bool {
    if stmt.negated || !stmt.redirects.is_empty() || stmt.terminator.is_some() {
        return false;
    }
    match &stmt.command {
        Command::Binary(_) => command_is_pipeline_of_compound_groups(&stmt.command),
        Command::Compound(CompoundCommand::BraceGroup(_) | CompoundCommand::Subshell(_)) => true,
        _ => false,
    }
}

pub(super) fn restore_raw_case_terminator_suffix_comments(
    rendered: &mut String,
    rendered_start: usize,
    raw: &str,
) {
    let comments = raw_case_terminator_suffix_comments_by_line(raw);
    if comments.iter().all(Option::is_none) || rendered_start >= rendered.len() {
        return;
    }

    let mut body = rendered[rendered_start..].to_string();
    let mut search_start = 0usize;
    for comment in comments {
        let Some((line_start, line_end)) =
            next_uncommented_case_terminator_line(&body, search_start)
        else {
            break;
        };
        if let Some(comment) = comment {
            let insert_at = line_end;
            body.insert_str(insert_at, &format!(" {comment}"));
            search_start = line_start + (line_end - line_start) + comment.len() + 1;
        } else {
            search_start = line_end.saturating_add(1);
        }
    }

    rendered.truncate(rendered_start);
    rendered.push_str(&body);
}

pub(super) fn raw_case_terminator_suffix_comments_by_line(raw: &str) -> Vec<Option<String>> {
    raw.lines()
        .filter_map(|line| {
            if !case_terminator_text_appears(line) {
                return None;
            }
            let comment = line.find('#').and_then(|comment_start| {
                let before_comment = line.get(..comment_start)?;
                if !case_terminator_text_appears(before_comment) {
                    return None;
                }
                Some(
                    line.get(comment_start..)?
                        .trim_end_matches([' ', '\t', '\r'])
                        .to_string(),
                )
            });
            Some(comment)
        })
        .collect()
}

pub(super) fn next_uncommented_case_terminator_line(
    body: &str,
    start: usize,
) -> Option<(usize, usize)> {
    let mut offset = start.min(body.len());
    while offset < body.len() {
        let relative_end = body[offset..]
            .find('\n')
            .unwrap_or(body.len().saturating_sub(offset));
        let line_end = offset + relative_end;
        let line = body.get(offset..line_end)?;
        if case_terminator_text_appears(line) && !line.contains('#') {
            return Some((offset, line_end));
        }
        offset = line_end.saturating_add(1);
    }
    None
}

pub(super) fn case_terminator_text_appears(text: &str) -> bool {
    text.contains(";;") || text.contains(";&") || text.contains(";;&")
}

pub(super) fn normalize_backtick_body_escaped_dollars(body: &str) -> String {
    let mut normalized = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && chars.peek() == Some(&'\\') {
            chars.next();
            if chars.peek() == Some(&'$') {
                normalized.push('\\');
                normalized.push('$');
                chars.next();
                continue;
            }
            normalized.push('\\');
            normalized.push('\\');
            continue;
        }
        normalized.push(ch);
    }
    normalized
}

pub(super) fn push_command_substitution_inline_body(
    target: &mut String,
    body: &str,
    options: &ResolvedShellFormatOptions,
    inline_continuation_indent_levels: usize,
) {
    let expanded_pipeline_brace_group = expand_inline_pipeline_brace_group_body(body, options);
    let body = expanded_pipeline_brace_group.as_deref().unwrap_or(body);
    let adjusted_body = indent_inline_case_command_body(body, options).or_else(|| {
        indent_inline_pipeline_continuations(body, options, inline_continuation_indent_levels)
    });
    let body = adjusted_body.as_deref().unwrap_or(body);
    if body.starts_with('(') {
        target.push(' ');
    }
    if options.space_redirects() {
        target.push_str(body);
    } else {
        push_raw_shell_text_with_normalized_redirect_spacing(target, body);
    }
}

pub(super) fn expand_inline_pipeline_brace_group_body(
    body: &str,
    options: &ResolvedShellFormatOptions,
) -> Option<String> {
    if body.contains('\n') || !raw_body_contains_pipeline_multistatement_brace_group(body) {
        return None;
    }

    let parsed = shuck_parser::parser::Parser::with_dialect(body, options.dialect()).parse();
    if parsed.is_err() {
        return None;
    }

    let mut nested = String::new();
    format_nested_stmt_sequence_to_buf(body, &parsed.file.body, options, None, None, &mut nested)?;
    let trimmed = trim_trailing_line_endings(&nested);
    trimmed.contains('\n').then(|| trimmed.to_string())
}

pub(super) fn indent_inline_case_command_body(
    body: &str,
    options: &ResolvedShellFormatOptions,
) -> Option<String> {
    if !body.contains('\n') || !body.trim_start_matches([' ', '\t']).starts_with("case ") {
        return None;
    }

    let prefix = options.indent_prefix(1);
    let mut rendered = String::with_capacity(body.len() + prefix.len());
    let mut changed = false;
    for (index, line) in body.split('\n').enumerate() {
        if index > 0 {
            rendered.push('\n');
        }
        if index > 0 && !line.trim().is_empty() {
            rendered.push_str(&prefix);
            changed = true;
        }
        rendered.push_str(line);
    }
    changed.then_some(rendered)
}

pub(super) fn trim_inline_command_substitution_padding(body: &str) -> &str {
    body.trim_matches([' ', '\t'])
}

pub(super) fn indent_inline_pipeline_continuations(
    body: &str,
    options: &ResolvedShellFormatOptions,
    indent_levels: usize,
) -> Option<String> {
    if !body.contains('\n') {
        return None;
    }

    let unit = options.indent_prefix(1);
    let prefix = unit.repeat(indent_levels.max(1));
    let mut rendered = String::with_capacity(body.len() + prefix.len());
    let mut changed = false;
    let mut previous_ends_pipeline = false;
    let mut pipeline_comment_continuation = false;
    let mut continuation_indent: Option<String> = None;
    let mut quote = QuoteState::default();

    for (index, line) in body.split('\n').enumerate() {
        if index > 0 {
            rendered.push('\n');
        }
        let mut rendered_line = String::new();
        let used_continuation_indent = if let Some(indent) = continuation_indent.take()
            && !line.trim().is_empty()
        {
            rendered_line.push_str(&indent);
            rendered_line.push_str(line.trim_start_matches([' ', '\t']));
            changed = true;
            true
        } else {
            false
        };
        let continues_pipeline_operand = previous_ends_pipeline || pipeline_comment_continuation;
        if !used_continuation_indent
            && continues_pipeline_operand
            && !line.is_empty()
            && !line.starts_with([' ', '\t'])
        {
            rendered_line.push_str(&prefix);
            rendered_line.push_str(line);
            changed = true;
        } else if !used_continuation_indent
            && continues_pipeline_operand
            && indent_levels > 1
            && !line.trim().is_empty()
            && line_leading_shell_indent(line) != prefix
        {
            rendered_line.push_str(&prefix);
            rendered_line.push_str(line.trim_start_matches([' ', '\t']));
            changed = true;
        } else if !used_continuation_indent {
            rendered_line.push_str(line);
        }

        rendered.push_str(&rendered_line);
        let line_is_pipeline_comment = continues_pipeline_operand
            && rendered_line
                .trim_start_matches([' ', '\t'])
                .starts_with('#');
        let line_continues = line_without_continuation_backslash(&rendered_line).is_some();
        quote.scan_line(&rendered_line);
        previous_ends_pipeline = line_ends_with_raw_continuation_operator(&rendered_line);
        pipeline_comment_continuation = line_is_pipeline_comment;
        continuation_indent = line_continues.then(|| {
            let indent = line_leading_shell_indent(&rendered_line);
            if quote.in_multiline_literal() || used_continuation_indent {
                indent.to_string()
            } else {
                source_indent_plus_one_unit(indent, options)
            }
        });
    }

    changed.then_some(rendered)
}

pub(super) fn line_ends_with_pipeline_operator(line: &str) -> bool {
    let trimmed = line.trim_end_matches([' ', '\t', '\r']);
    trimmed.ends_with("|&") || (trimmed.ends_with('|') && !trimmed.ends_with("||"))
}

pub(super) fn line_ends_with_raw_continuation_operator(line: &str) -> bool {
    let code = trailing_comment_start(line)
        .map(|comment_start| &line[..comment_start])
        .unwrap_or(line);
    let trimmed = code.trim_end_matches([' ', '\t', '\r']);
    line_ends_with_pipeline_operator(trimmed) || trimmed.ends_with("&&") || trimmed.ends_with("||")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CommandSubstitutionLayout {
    Inline,
    InlineContinued,
    InlineSourceIndented,
    Block,
}

pub(super) fn command_substitution_layout(
    raw: Option<&str>,
    body: &shuck_ast::StmtSeq,
    facts: Option<&FormatterFacts<'_>>,
    source: &str,
    dialect: shuck_parser::ShellDialect,
    force_block: bool,
    allow_source_indented_inline: bool,
) -> CommandSubstitutionLayout {
    if force_block {
        return CommandSubstitutionLayout::Block;
    }

    if stmt_seq_has_heredoc(facts, body) {
        return CommandSubstitutionLayout::Block;
    }

    if let Some(raw) = raw {
        if command_substitution_source_starts_with_body_line(raw) {
            return CommandSubstitutionLayout::Block;
        }
        if command_substitution_source_closes_on_own_line(raw) {
            return CommandSubstitutionLayout::Block;
        }
        if command_substitution_source_parses_as_multiple_statements(raw, dialect) {
            return CommandSubstitutionLayout::Block;
        }
        if command_substitution_source_prefers_continued_inline_body(raw) {
            return CommandSubstitutionLayout::InlineContinued;
        }
        if allow_source_indented_inline && raw.contains('\n') {
            return CommandSubstitutionLayout::InlineSourceIndented;
        }
    }

    if body.len() > 1
        || body
            .span
            .slice(source)
            .trim_start_matches([' ', '\t', '\r'])
            .starts_with('\n')
    {
        CommandSubstitutionLayout::Block
    } else {
        CommandSubstitutionLayout::Inline
    }
}

pub(super) fn command_substitution_source_parses_as_multiple_statements(
    raw: &str,
    dialect: shuck_parser::ShellDialect,
) -> bool {
    if raw.contains('\n') || !raw.contains(';') {
        return false;
    }

    let Some(body) = raw_dollar_command_substitution_body(raw) else {
        return false;
    };
    let body = body.trim();
    if body.is_empty() {
        return false;
    }

    let parsed = shuck_parser::parser::Parser::with_dialect(body, dialect).parse();
    !parsed.is_err() && parsed.file.body.len() > 1
}

pub(super) fn raw_dollar_command_substitution_body(raw: &str) -> Option<&str> {
    raw.strip_prefix("$(")?;
    let close_offset = matching_raw_command_substitution_close(raw, 2)?;
    raw.get(2..close_offset)
}

pub(super) fn raw_dollar_command_substitution_slice(raw: &str) -> Option<&str> {
    raw.strip_prefix("$(")?;
    let close_offset = matching_raw_command_substitution_close(raw, 2)?;
    raw.get(..close_offset + 1)
}

pub(super) fn command_substitution_source_starts_with_body_line(raw: &str) -> bool {
    if raw.starts_with(['\n', '\r']) {
        return true;
    }
    raw.strip_prefix("$(")
        .is_some_and(|after_open| after_open.starts_with(['\n', '\r']))
}

pub(super) fn command_substitution_source_closes_on_own_line(raw: &str) -> bool {
    substitution_source_closes_on_own_line(raw)
}

pub(super) fn push_inline_raw_command_substitution_as_block(
    target: &mut String,
    raw: &str,
    options: &ResolvedShellFormatOptions,
) -> bool {
    let Some(after_open) = raw.strip_prefix("$(") else {
        return false;
    };
    if after_open.starts_with(['\n', '\r']) || !command_substitution_source_closes_on_own_line(raw)
    {
        return false;
    }

    let Some(close_offset) = raw.rfind(')') else {
        return false;
    };
    let Some(close_line_start) = raw[..close_offset].rfind('\n').map(|index| index + 1) else {
        return false;
    };
    let Some(body_source) = raw.get(2..close_line_start) else {
        return false;
    };
    let body_source = body_source.trim_end_matches(['\n', '\r']);
    if body_source.trim().is_empty() {
        target.push_str("$()");
        return true;
    }

    let nested = normalize_inline_raw_command_substitution_body(body_source, options);
    target.push_str("$(\n");
    push_indented_rendered_block(target, &nested, options, 1);
    target.push_str("\n)");
    true
}

pub(super) fn push_inline_raw_command_substitution_with_normalized_body(
    target: &mut String,
    raw: &str,
    options: &ResolvedShellFormatOptions,
) -> bool {
    if command_substitution_source_starts_with_body_line(raw)
        || command_substitution_source_closes_on_own_line(raw)
    {
        return false;
    }
    let Some(body_source) = raw_dollar_command_substitution_body(raw) else {
        return false;
    };
    if !body_source.contains('\n') {
        return false;
    }

    let body_source = body_source.trim_start_matches([' ', '\t', '\r']);
    if !body_source.starts_with('(') {
        return false;
    }

    let nested = normalize_inline_raw_command_substitution_body_preserving_nested_comments(
        body_source,
        options,
    );
    target.push_str("$(");
    if nested.starts_with('(') {
        target.push(' ');
    }
    target.push_str(&nested);
    target.push(')');
    true
}

pub(super) fn normalize_inline_raw_command_substitution_body(
    body_source: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    normalize_inline_raw_command_substitution_body_with_options(body_source, options, false)
}

pub(super) fn normalize_inline_raw_command_substitution_body_preserving_nested_comments(
    body_source: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    normalize_inline_raw_command_substitution_body_with_options(body_source, options, true)
}

pub(super) fn normalize_inline_raw_command_substitution_body_with_options(
    body_source: &str,
    options: &ResolvedShellFormatOptions,
    preserve_nested_comment_indent: bool,
) -> String {
    let normalized = normalize_raw_pipeline_continuations(body_source);
    let normalized_pipeline_continuation = normalized.is_some();
    let body_source = normalized.as_deref().unwrap_or(body_source);
    let normalized_comment_continuations =
        normalize_continuations_before_comment_lines(body_source);
    let body_source = normalized_comment_continuations
        .as_deref()
        .unwrap_or(body_source);
    let normalized_close_continuations =
        normalize_continuations_before_substitution_close_lines(body_source);
    let body_source = normalized_close_continuations
        .as_deref()
        .unwrap_or(body_source);
    let lines = body_source.lines().map(str::to_string).collect::<Vec<_>>();
    let source_base_indent = inline_raw_body_source_base_indent(&lines);

    let mut rendered = String::new();
    let mut previous_pipeline_indent_units: Option<usize> = None;
    let mut continuation_indent_units: Option<usize> = None;
    let mut pipeline_compounds = Vec::<InlinePipelineCompound>::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            rendered.push('\n');
        }
        let content = line.trim_start_matches([' ', '\t']);
        if content.trim().is_empty() {
            previous_pipeline_indent_units = None;
            continuation_indent_units = None;
            continue;
        }

        let carried_pipeline_indent = previous_pipeline_indent_units;
        let pipeline_base_units = pipeline_compounds
            .last()
            .map(|compound| compound.base_units)
            .unwrap_or(0);
        let relative_source_indent =
            inline_raw_body_relative_source_indent(line, index, source_base_indent.as_deref());
        let relative_indent = if content.starts_with('#')
            && carried_pipeline_indent.is_none()
            && pipeline_compounds.is_empty()
            && (!preserve_nested_comment_indent || relative_source_indent.is_empty())
        {
            ""
        } else {
            relative_source_indent
        };
        let mut indent_units = pipeline_base_units + raw_indent_units(relative_indent, options);
        if let Some(previous_units) = carried_pipeline_indent {
            let extra_units = usize::from(!normalized_pipeline_continuation);
            indent_units = indent_units.max(previous_units + extra_units);
        }
        let mut used_continuation_indent = false;
        if let Some(units) = continuation_indent_units.take()
            && !content.starts_with('#')
        {
            indent_units = units;
            used_continuation_indent = true;
        }

        rendered.extend(std::iter::repeat_n('\t', indent_units));
        push_raw_shell_line_with_normalized_redirect_spacing(&mut rendered, content);
        let line_is_pipeline_continuation_stage = carried_pipeline_indent.is_some();
        if content.starts_with('#') {
            previous_pipeline_indent_units = carried_pipeline_indent;
        } else {
            previous_pipeline_indent_units =
                line_ends_with_raw_continuation_operator(content).then_some(indent_units);
            if line_without_continuation_backslash(content).is_some() {
                continuation_indent_units = Some(if used_continuation_indent {
                    indent_units
                } else {
                    indent_units + 1
                });
            } else {
                continuation_indent_units = None;
            }
        }
        if let Some(close_keyword) = raw_compound_close_keyword(content)
            && (line_is_pipeline_continuation_stage || !pipeline_compounds.is_empty())
        {
            pipeline_compounds.push(InlinePipelineCompound {
                close_keyword,
                base_units: if line_is_pipeline_continuation_stage {
                    indent_units
                } else {
                    pipeline_base_units
                },
            });
        }
        if pipeline_compounds
            .last()
            .is_some_and(|compound| raw_line_closes_compound(content, compound.close_keyword))
        {
            pipeline_compounds.pop();
        }
    }

    rendered
}

pub(super) struct InlinePipelineCompound {
    close_keyword: &'static str,
    base_units: usize,
}

pub(super) fn inline_raw_body_source_base_indent(lines: &[String]) -> Option<String> {
    let mut common: Option<String> = None;
    for line in lines.iter().skip(1) {
        if line.trim_matches([' ', '\t', '\r']).is_empty() {
            continue;
        }
        let indent = line_leading_shell_indent(line);
        if refine_common_indent(&mut common, indent) {
            return None;
        }
    }
    common
}

pub(super) fn inline_raw_body_relative_source_indent<'a>(
    line: &'a str,
    index: usize,
    source_base_indent: Option<&str>,
) -> &'a str {
    let indent = line_leading_shell_indent(line);
    if index == 0 {
        return indent;
    }
    let Some(source_base_indent) = source_base_indent else {
        return indent;
    };
    indent.strip_prefix(source_base_indent).unwrap_or("")
}

pub(super) fn command_substitution_source_prefers_continued_inline_body(raw: &str) -> bool {
    let Some(after_open) = raw.strip_prefix("$(") else {
        return false;
    };
    if after_open.starts_with(['\n', '\r']) {
        return false;
    }

    raw.lines()
        .any(|line| line.trim_end_matches([' ', '\t', '\r']).ends_with('\\'))
}

pub(super) fn push_raw_block_command_substitution_without_outer_indent(
    target: &mut String,
    raw: &str,
    source: &str,
    start_offset: usize,
    options: &ResolvedShellFormatOptions,
) {
    let normalized_pipeline = normalize_raw_pipeline_continuations(raw);
    let normalized_pipeline_continuation = normalized_pipeline.is_some();
    let raw = normalized_pipeline.as_deref().unwrap_or(raw);
    let normalized_comment_continuations = normalize_continuations_before_comment_lines(raw);
    let raw = normalized_comment_continuations.as_deref().unwrap_or(raw);
    let normalized_close_continuations =
        normalize_continuations_before_substitution_close_lines(raw);
    let raw = normalized_close_continuations.as_deref().unwrap_or(raw);
    let outer_indent = line_indent_before_source_offset(source, start_offset).unwrap_or("");
    let raw_lines = raw.split('\n').collect::<Vec<_>>();
    let Some((first, lines)) = raw_lines.split_first() else {
        return;
    };
    target.push_str(first);
    let mut body_indent: Option<String> = None;
    let mut previous_pipeline_indent: Option<String> = None;
    let mut continuation_indent: Option<String> = None;
    let mut compound_indents = RawCompoundIndentState::default();
    let mut quote = QuoteState::default();
    for (line_index, line) in lines.iter().enumerate() {
        let line = *line;
        target.push('\n');
        if quote.in_multiline_literal() {
            target.push_str(line);
            quote.scan_line(line);
            let (indent, content) = raw_line_parts(line);
            previous_pipeline_indent = if content.trim().is_empty() {
                None
            } else if line_ends_with_raw_continuation_operator(line) {
                Some(indent.to_string())
            } else {
                None
            };
            continue;
        }

        let mut line = strip_outer_indent_or_one_unit(line, outer_indent, options).to_string();
        let source_indent_for_compound_shift = line_leading_shell_indent(&line).to_string();
        if let Some(shifted) = compound_indents.shifted_line(&line, options) {
            line = shifted;
        }
        let carried_pipeline_indent = previous_pipeline_indent.clone();
        let mut force_preserve_line_indent = false;
        let (indent, content) = raw_line_parts(&line);
        if let Some(previous_indent) = previous_pipeline_indent.as_deref()
            && !content.trim().is_empty()
            && !raw_line_closes_substitution_wrapper(content)
        {
            let desired_indent = if normalized_pipeline_continuation {
                previous_indent.to_string()
            } else {
                source_indent_plus_one_unit(previous_indent, options)
            };
            if raw_indent_units(indent, options) < raw_indent_units(&desired_indent, options) {
                line = format!("{desired_indent}{content}");
                force_preserve_line_indent = true;
            }
        }
        let (indent, content) = raw_line_parts(&line);
        let in_compound_body = compound_indents.in_body(content);
        if let Some(child_indent) =
            compound_indents.child_indent_if_underindented(indent, content, options)
        {
            line = format!("{child_indent}{content}");
            force_preserve_line_indent = true;
        }
        let (indent, content) = raw_line_parts(&line);
        let mut forced_rendered_indent = None;
        let mut used_continuation_indent = false;
        if let Some(previous_indent) = continuation_indent.take()
            && !content.trim().is_empty()
            && !content.starts_with('#')
            && !raw_line_closes_substitution_wrapper(content)
        {
            forced_rendered_indent = Some(previous_indent);
            force_preserve_line_indent = true;
            used_continuation_indent = true;
        }
        if compound_indents.comments.len() > 1 && compound_indents.closes_last(content) {
            force_preserve_line_indent = true;
        }
        let closes_substitution_wrapper = raw_line_closes_substitution_wrapper(content)
            && raw_block_line_is_outer_substitution_close(lines, line_index);
        let leading_block_comment = body_indent.is_none() && content.starts_with('#');
        if body_indent.is_none()
            && !content.trim().is_empty()
            && !content.starts_with('#')
            && !closes_substitution_wrapper
        {
            body_indent = Some(indent.to_string());
        }
        let is_pipeline_continuation =
            carried_pipeline_indent.is_some() && !content.trim().is_empty();
        let body_indent_for_line =
            if force_preserve_line_indent || is_pipeline_continuation || in_compound_body {
                None
            } else if leading_block_comment {
                Some("")
            } else {
                body_indent.as_deref()
            };
        let rendered_indent = if closes_substitution_wrapper {
            push_raw_shell_line_with_rendered_indent(target, &line, options, "");
            String::new()
        } else if let Some(rendered_indent) = forced_rendered_indent.as_deref() {
            push_raw_shell_line_with_rendered_indent(target, &line, options, rendered_indent);
            rendered_indent.to_string()
        } else {
            push_raw_shell_line_with_normalized_source_indent(
                target,
                &line,
                options,
                body_indent_for_line,
            );
            rendered_raw_shell_indent_for_line(indent, content, body_indent_for_line, options)
        };
        let line_is_pipeline_continuation_stage = carried_pipeline_indent.is_some();
        previous_pipeline_indent = if content.trim().is_empty() {
            None
        } else if content.starts_with('#') {
            carried_pipeline_indent
        } else if line_ends_with_raw_continuation_operator(&line) {
            carried_pipeline_indent.or_else(|| Some(rendered_indent.clone()))
        } else {
            None
        };
        let line_continues = line_without_continuation_backslash(&line).is_some();
        quote.scan_line(&line);
        continuation_indent = if line_continues && !content.starts_with('#') {
            Some(if used_continuation_indent {
                rendered_indent.clone()
            } else {
                source_indent_plus_one_unit(&rendered_indent, options)
            })
        } else {
            None
        };
        compound_indents.update_line(
            content,
            &source_indent_for_compound_shift,
            &rendered_indent,
            indent,
            line_is_pipeline_continuation_stage,
            options,
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_process_substitution(
    rendered: &mut String,
    body: &shuck_ast::StmtSeq,
    is_input: bool,
    span: shuck_ast::Span,
    source: &str,
    options: &ResolvedShellFormatOptions,
    multiline: bool,
    raw: Option<&str>,
    facts: Option<&FormatterFacts<'_>>,
) -> Option<()> {
    let has_heredoc = stmt_seq_has_heredoc(facts, body);
    let mut nested = String::new();
    format_nested_stmt_sequence_to_buf(
        source,
        body,
        options,
        facts,
        span.end.offset.checked_sub(1),
        &mut nested,
    )?;

    let prefix = if is_input { '<' } else { '>' };
    let trimmed = trim_trailing_line_endings(&nested);
    if trimmed.is_empty() {
        rendered.push(prefix);
        rendered.push_str("()");
        return Some(());
    }

    let rendered_multiline = trimmed.contains('\n');
    if multiline || has_heredoc || rendered_multiline {
        if rendered_multiline
            && !has_heredoc
            && raw.is_some_and(process_substitution_source_starts_with_inline_brace_group)
        {
            rendered.push(prefix);
            rendered.push('(');
            rendered.push_str(trimmed);
            rendered.push(')');
        } else if let Some(raw) = raw
            && process_substitution_source_starts_with_body_line(raw)
            && raw.contains('\n')
            && !substitution_source_closes_on_own_line(raw)
        {
            rendered.push(prefix);
            rendered.push('(');
            push_source_indented_inline_command_substitution(rendered, trimmed, raw, options);
            rendered.push(')');
        } else {
            let outer_levels =
                source_indent_units_before_offset(source, span.start.offset, options);
            rendered.push(prefix);
            rendered.push_str("(\n");
            push_indented_rendered_block(rendered, trimmed, options, outer_levels + 1);
            rendered.push('\n');
            options.push_indent_units(rendered, outer_levels);
            rendered.push(')');
        }
    } else {
        rendered.push(prefix);
        rendered.push('(');
        rendered.push_str(trimmed);
        rendered.push(')');
    }

    Some(())
}

pub(super) fn process_substitution_source_starts_with_inline_brace_group(raw: &str) -> bool {
    raw.get(2..).is_some_and(|body| {
        (raw.starts_with("<(") || raw.starts_with(">("))
            && !body.starts_with(['\n', '\r'])
            && body.trim_start_matches([' ', '\t']).starts_with('{')
    })
}

pub(super) fn process_substitution_source_starts_with_body_line(raw: &str) -> bool {
    raw.get(2..).is_some_and(|body| {
        (raw.starts_with("<(") || raw.starts_with(">(")) && !body.starts_with('\n')
    })
}

pub(super) fn process_substitution_source_opens_to_body_line(raw: &str) -> bool {
    raw.get(2..).is_some_and(|body| {
        (raw.starts_with("<(") || raw.starts_with(">(")) && body.starts_with(['\n', '\r'])
    })
}

pub(super) fn substitution_source_closes_on_own_line(raw: &str) -> bool {
    let Some(close_offset) = raw.rfind(')') else {
        return false;
    };
    let line_start = raw[..close_offset]
        .rfind('\n')
        .map_or(0, |newline| newline.saturating_add(1));
    line_start > 0 && raw[line_start..close_offset].trim().is_empty()
}

pub(super) fn trim_trailing_line_endings(rendered: &str) -> &str {
    rendered.trim_end_matches(&['\r', '\n'][..])
}

pub(super) fn push_source_indented_inline_command_substitution(
    target: &mut String,
    rendered: &str,
    raw: &str,
    options: &ResolvedShellFormatOptions,
) {
    let raw_indents = raw
        .lines()
        .skip(1)
        .map(line_leading_shell_indent)
        .map(|indent| normalized_source_inline_indent(indent, options))
        .collect::<Vec<_>>();
    let fallback_indent = raw_indents.first().map(String::as_str).unwrap_or("");
    for (index, line) in rendered.lines().enumerate() {
        if index > 0 {
            target.push('\n');
            let indent = raw_indents
                .get(index - 1)
                .map(String::as_str)
                .unwrap_or(fallback_indent);
            target.push_str(indent);
        }
        if index == 0 {
            target.push_str(line);
        } else {
            target.push_str(line.trim_start_matches([' ', '\t']));
        }
    }
}

pub(super) fn normalized_source_inline_indent(
    indent: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    match options.indent_style() {
        IndentStyle::Tab if indent.chars().all(|ch| ch == ' ') => {
            let unit = usize::from(options.indent_width()).clamp(1, 4);
            if indent.len().is_multiple_of(unit) {
                "\t".repeat(indent.len() / unit)
            } else {
                indent.to_string()
            }
        }
        IndentStyle::Space if indent.chars().all(|ch| ch == '\t') => {
            " ".repeat(indent.len() * usize::from(options.indent_width()))
        }
        _ => indent.to_string(),
    }
}

pub(super) fn normalized_raw_shell_indent(
    indent: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    match options.indent_style() {
        IndentStyle::Tab if !indent.is_empty() && indent.chars().all(|ch| ch == ' ') => {
            let unit = usize::from(options.indent_width()).clamp(1, 4);
            "\t".repeat(indent.len().div_ceil(unit))
        }
        _ => normalized_source_inline_indent(indent, options),
    }
}

pub(super) fn push_indented_rendered_block(
    target: &mut String,
    rendered: &str,
    options: &ResolvedShellFormatOptions,
    levels: usize,
) {
    let prefix = options.indent_prefix(levels);
    let normalized_literal_continuations =
        normalize_literal_continuation_indent_for_block(rendered);
    let rendered = normalized_literal_continuations
        .as_deref()
        .unwrap_or(rendered);
    let common_source_indent = common_rendered_block_indent(rendered, options);

    let mut active_heredoc: Option<CommandSubstitutionHeredocIndent> = None;
    for (index, line) in rendered.lines().enumerate() {
        if index > 0 {
            target.push('\n');
        }

        if let Some(heredoc) = active_heredoc.as_ref() {
            let closes = heredoc_line_closes_command_substitution_heredoc(line, heredoc);
            if heredoc.strip_tabs {
                if closes {
                    target.push_str(&prefix);
                    target.push_str(&heredoc.command_indent);
                    target.push_str(line.trim_start_matches('\t'));
                    active_heredoc = None;
                    continue;
                }
                if line_needs_command_substitution_indent(line, options) {
                    target.push_str(&prefix);
                }
            }
            target.push_str(line);
            if closes {
                active_heredoc = None;
            }
            continue;
        }

        let line = strip_common_rendered_block_indent(line, &common_source_indent);
        if line_needs_command_substitution_indent(line, options) {
            target.push_str(&prefix);
        }
        target.push_str(line);
        active_heredoc = command_substitution_heredoc_indent(line);
    }
}

pub(super) fn normalize_literal_continuation_indent_for_block(rendered: &str) -> Option<String> {
    if !rendered.contains('\n') {
        return None;
    }

    let mut quote = QuoteState::default();
    let mut continuation_indent: Option<String> = None;
    let mut normalized = String::with_capacity(rendered.len());
    let mut changed = false;

    for (index, line) in rendered.split('\n').enumerate() {
        if index > 0 {
            normalized.push('\n');
        }

        let mut line = line.to_string();
        if let Some(indent) = continuation_indent.take()
            && !line.trim().is_empty()
        {
            let content = line.trim_start_matches([' ', '\t']);
            if line_leading_shell_indent(&line) != indent {
                line = format!("{indent}{content}");
                changed = true;
            }
        }

        let line_continues = line_without_continuation_backslash(&line).is_some();
        quote.scan_line(&line);
        continuation_indent = (line_continues && quote.in_multiline_literal())
            .then(|| line_leading_shell_indent(&line).to_string());
        normalized.push_str(&line);
    }

    changed.then_some(normalized)
}

pub(super) fn common_rendered_block_indent(
    rendered: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    let mut active_heredoc: Option<CommandSubstitutionHeredocIndent> = None;
    let mut common: Option<String> = None;

    for line in rendered.lines() {
        if let Some(heredoc) = active_heredoc.as_ref() {
            if heredoc_line_closes_command_substitution_heredoc(line, heredoc) {
                active_heredoc = None;
            }
            continue;
        }

        if line_needs_command_substitution_indent(line, options) {
            let indent = line_leading_shell_indent(line);
            if indent.is_empty() {
                return String::new();
            }
            if refine_common_indent(&mut common, indent) {
                return String::new();
            }
        }

        active_heredoc = command_substitution_heredoc_indent(line);
    }

    common.unwrap_or_default()
}

pub(super) fn strip_common_rendered_block_indent<'a>(
    line: &'a str,
    common_indent: &str,
) -> &'a str {
    if common_indent.is_empty() {
        line
    } else {
        line.strip_prefix(common_indent).unwrap_or(line)
    }
}

#[derive(Debug, Clone)]
pub(super) struct CommandSubstitutionHeredocIndent {
    delimiter: String,
    strip_tabs: bool,
    command_indent: String,
}

pub(super) fn command_substitution_heredoc_indent(
    line: &str,
) -> Option<CommandSubstitutionHeredocIndent> {
    let start = heredoc_start(line)?;
    Some(CommandSubstitutionHeredocIndent {
        delimiter: start.delimiter.to_string(),
        strip_tabs: start.strip_tabs,
        command_indent: line_leading_shell_indent(line).to_string(),
    })
}

pub(super) fn heredoc_line_closes_command_substitution_heredoc(
    line: &str,
    heredoc: &CommandSubstitutionHeredocIndent,
) -> bool {
    if heredoc.strip_tabs {
        line.trim_start_matches('\t') == heredoc.delimiter
    } else {
        line == heredoc.delimiter
    }
}

pub(super) fn line_needs_command_substitution_indent(
    line: &str,
    options: &ResolvedShellFormatOptions,
) -> bool {
    if line.is_empty() {
        return false;
    }

    match options.indent_style() {
        // Leave literal multiline string continuation lines alone. Formatter-
        // produced shell indentation already uses tabs in this mode.
        IndentStyle::Tab => !line.starts_with(' '),
        IndentStyle::Space => true,
    }
}
