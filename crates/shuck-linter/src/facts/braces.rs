fn build_literal_brace_spans(
    words: &[WordFact<'_>],
    commands: &[CommandFact<'_>],
    source: &str,
    heredoc_ranges: &[TextRange],
) -> Vec<Span> {
    let mut spans = Vec::new();

    for fact in words {
        if fact.expansion_context() == Some(ExpansionContext::RegexOperand) {
            continue;
        }

        let is_find_exec_placeholder_word = is_find_exec_placeholder_word(commands, fact, source);
        let is_xargs_replacement_word = is_xargs_replacement_word(commands, fact, source);
        let direct_spans = fact
            .word()
            .brace_syntax()
            .iter()
            .copied()
            .filter(|brace| brace.quote_context == BraceQuoteContext::Unquoted)
            .filter(|brace| !literal_brace_syntax_looks_like_active_expansion(*brace, source))
            .filter(|brace| {
                matches!(
                    brace.kind,
                    BraceSyntaxKind::Literal | BraceSyntaxKind::TemplatePlaceholder
                ) || brace_syntax_with_whitespace_is_literal(*brace, source)
            })
            .filter(|brace| {
                brace.span.slice(source) != "{}"
                    && !brace_span_has_escaped_dollar_prefix(brace.span, source)
                    && !is_find_exec_placeholder_word
                    && !is_xargs_replacement_word
            })
            .flat_map(|brace| brace_character_spans(brace.span, source))
            .filter(|span| {
                !span_inside_nested_escaped_parameter_template(fact.word(), *span, source)
            })
            .filter(|span| {
                !brace_span_is_plain_parameter_expansion_edge(fact.word(), *span, source)
            })
            .filter(|span| !word_span_is_inside_command_substitution(fact, *span))
            .collect::<Vec<_>>();
        spans.extend(direct_spans);

        if !is_find_exec_placeholder_word && !is_xargs_replacement_word {
            let unclassified = unclassified_literal_brace_spans(fact.word(), source)
                .into_iter()
                .filter(|span| {
                    !span_inside_nested_escaped_parameter_template(fact.word(), *span, source)
                })
                .filter(|span| {
                    !brace_span_is_plain_parameter_expansion_edge(fact.word(), *span, source)
                })
                .filter(|span| !word_span_is_inside_command_substitution(fact, *span))
                .collect::<Vec<_>>();
            spans.extend(unclassified);
            let escaped = escaped_parameter_expansion_brace_edge_spans(fact.word(), source)
                .into_iter()
                .filter(|span| {
                    !span_inside_nested_escaped_parameter_template(fact.word(), *span, source)
                })
                .filter(|span| {
                    !brace_span_is_plain_parameter_expansion_edge(fact.word(), *span, source)
                })
                .filter(|span| !word_span_is_inside_command_substitution(fact, *span))
                .collect::<Vec<_>>();
            spans.extend(escaped);
        }
    }

    spans.extend(uncovered_command_brace_spans(
        commands,
        source,
        heredoc_ranges,
    ));
    spans.extend(unmatched_command_substitution_brace_spans(
        commands,
        source,
        heredoc_ranges,
    ));
    spans.retain(|span| !span_is_plain_parameter_expansion_edge_in_source(*span, source));
    spans.retain(|span| !span_is_active_brace_expansion_edge_in_source(*span, source));
    spans
}

fn word_span_is_inside_command_substitution(fact: &WordFact<'_>, span: Span) -> bool {
    fact.command_substitution_spans()
        .iter()
        .copied()
        .any(|substitution| contains_span(substitution, span))
}

fn brace_span_is_plain_parameter_expansion_edge(word: &Word, span: Span, source: &str) -> bool {
    if span.start.offset < word.span.start.offset || span.start.offset >= word.span.end.offset {
        return false;
    }

    let text = word.span.slice(source);
    let relative_offset = span.start.offset - word.span.start.offset;
    let mut index = 0usize;

    while index < text.len() {
        if text[index..].starts_with("${")
            && !has_odd_backslash_run_before(text, index)
            && let Some(end_offset) = find_runtime_parameter_closing_brace(text, index)
        {
            let open_brace_offset = index + '$'.len_utf8();
            let close_brace_offset = end_offset.saturating_sub('}'.len_utf8());
            if relative_offset == open_brace_offset || relative_offset == close_brace_offset {
                return true;
            }
            index = end_offset;
            continue;
        }

        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        let ch_len = ch.len_utf8();
        if ch == '\\' {
            index += ch_len;
            if let Some(escaped) = text[index..].chars().next() {
                index += escaped.len_utf8();
            }
            continue;
        }

        index += ch_len;
    }

    false
}

fn span_is_plain_parameter_expansion_edge_in_source(span: Span, source: &str) -> bool {
    let target_offset = span.start.offset;
    let mut index = 0usize;

    while index < source.len() {
        if source[index..].starts_with("${")
            && !has_odd_backslash_run_before(source, index)
            && let Some(end_offset) = find_runtime_parameter_closing_brace(source, index)
        {
            let open_brace_offset = index + '$'.len_utf8();
            let close_brace_offset = end_offset.saturating_sub('}'.len_utf8());
            if target_offset == open_brace_offset || target_offset == close_brace_offset {
                return true;
            }
            index = end_offset;
            continue;
        }

        let Some(ch) = source[index..].chars().next() else {
            break;
        };
        let ch_len = ch.len_utf8();
        if ch == '\\' {
            index += ch_len;
            if let Some(escaped) = source[index..].chars().next() {
                index += escaped.len_utf8();
            }
            continue;
        }

        index += ch_len;
    }

    false
}

fn span_is_active_brace_expansion_edge_in_source(span: Span, source: &str) -> bool {
    let offset = span.start.offset;
    let Some(ch) = source[offset..].chars().next() else {
        return false;
    };

    let candidate = match ch {
        '{' => source[offset..]
            .find('}')
            .map(|relative_end| &source[offset..=offset + relative_end]),
        '}' => source[..offset]
            .rfind('{')
            .map(|start| &source[start..=offset]),
        _ => None,
    };

    candidate.is_some_and(|text| {
        brace_text_has_unescaped_comma_or_sequence(text)
            && text[1..text.len() - 1]
                .chars()
                .all(|candidate| !candidate.is_whitespace())
    })
}

fn is_find_exec_placeholder_word(
    commands: &[CommandFact<'_>],
    fact: &WordFact<'_>,
    source: &str,
) -> bool {
    if !word_is_empty_brace_pair_variant(fact.word(), source) {
        return false;
    }
    if fact.expansion_context() != Some(ExpansionContext::CommandArgument) {
        return false;
    }

    let command = &commands[fact.command_id().index()];
    if command.has_wrapper(WrapperKind::FindExec) || command.has_wrapper(WrapperKind::FindExecDir) {
        return true;
    }

    commands.iter().any(|command| {
        command.stmt().span.start.offset <= fact.span().start.offset
            && command.stmt().span.end.offset >= fact.span().end.offset
            && is_find_exec_command(command, source)
    }) || line_has_find_exec_placeholder_context(source, fact.span())
}

fn is_find_exec_command(command: &CommandFact<'_>, source: &str) -> bool {
    let is_find = command.static_utility_name_is("find")
        || command.body_name_word().is_some_and(|name_word| {
            name_word
                .span
                .slice(source)
                .rsplit('/')
                .next()
                .is_some_and(|name| name == "find")
        });
    if !is_find {
        return false;
    }

    let has_exec_flag = command.body_args().iter().any(|arg| {
        matches!(
            arg.span.slice(source),
            "-exec" | "-execdir" | "-ok" | "-okdir"
        )
    });
    let has_exec_terminator = command
        .body_args()
        .iter()
        .any(|arg| matches!(arg.span.slice(source), "+" | "\\;"));

    has_exec_flag && has_exec_terminator
}

fn line_has_find_exec_placeholder_context(source: &str, brace_span: Span) -> bool {
    let Some(line_text) = source.lines().nth(brace_span.start.line.saturating_sub(1)) else {
        return false;
    };
    let line_start_offset = source
        .lines()
        .take(brace_span.start.line.saturating_sub(1))
        .map(|line| line.len() + '\n'.len_utf8())
        .sum::<usize>();
    let Some(relative_start) = brace_span.start.offset.checked_sub(line_start_offset) else {
        return false;
    };
    let Some(relative_end) = brace_span.end.offset.checked_sub(line_start_offset) else {
        return false;
    };
    if relative_end > line_text.len() {
        return false;
    }

    let prefix = &line_text[..relative_start];
    let suffix = &line_text[relative_end..];
    let first_word = shellish_words(prefix).into_iter().next();
    let has_exec_flag_before = shellish_words(prefix)
        .into_iter()
        .any(|word| matches!(word, "-exec" | "-execdir" | "-ok" | "-okdir"));
    let has_exec_terminator_after = shellish_words(suffix)
        .into_iter()
        .any(|word| matches!(word, "+" | "\\;"));

    first_word
        .and_then(|word| word.rsplit('/').next())
        .is_some_and(|word| word == "find")
        && has_exec_flag_before
        && has_exec_terminator_after
}

fn is_xargs_replacement_word(
    commands: &[CommandFact<'_>],
    fact: &WordFact<'_>,
    source: &str,
) -> bool {
    if fact.expansion_context() != Some(ExpansionContext::CommandArgument) {
        return false;
    }

    let command = &commands[fact.command_id().index()];
    if !command.effective_name_is("xargs") {
        return false;
    }

    xargs_replacement_spans(command.body_args(), source)
        .into_iter()
        .any(|span| span == fact.word().span)
}

fn xargs_replacement_spans(args: &[&Word], source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut index = 0usize;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };

        if text == "--" {
            break;
        }

        if let Some(long) = text.strip_prefix("--") {
            if let Some(replacement) = long.strip_prefix("replace=") {
                if !replacement.is_empty() {
                    spans.push(word.span);
                }
                index += 1;
                continue;
            }

            if long == "replace" {
                let Some(next_word) = args.get(index + 1) else {
                    break;
                };
                spans.push(next_word.span);
                index += 2;
                continue;
            }

            let consume_next_argument = xargs_long_option_requires_separate_argument(long);
            index += 1;
            if consume_next_argument {
                index += 1;
            }
            continue;
        }

        if !text.starts_with('-') || text == "-" {
            break;
        }

        let mut chars = text[1..].chars().peekable();
        let mut consume_next_argument = false;

        while let Some(flag) = chars.next() {
            match flag {
                'i' => {
                    if chars.peek().is_some() {
                        spans.push(word.span);
                    }
                    break;
                }
                'I' => {
                    if chars.peek().is_some() {
                        spans.push(word.span);
                    } else {
                        let Some(next_word) = args.get(index + 1) else {
                            return spans;
                        };
                        spans.push(next_word.span);
                        consume_next_argument = true;
                    }
                    break;
                }
                _ => match xargs_short_option_argument_style(flag) {
                    XargsShortOptionArgumentStyle::None => {}
                    XargsShortOptionArgumentStyle::OptionalInlineOnly => break,
                    XargsShortOptionArgumentStyle::Required => {
                        if chars.peek().is_none() {
                            consume_next_argument = true;
                        }
                        break;
                    }
                },
            }
        }

        index += 1;
        if consume_next_argument {
            index += 1;
        }
    }

    spans
}

fn shellish_words(text: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut start = None;

    for (index, ch) in text.char_indices() {
        let is_word =
            ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '+' | '/' | '\\' | ';' | '.');
        if is_word {
            if start.is_none() {
                start = Some(index);
            }
        } else if let Some(word_start) = start.take() {
            words.push(&text[word_start..index]);
        }
    }

    if let Some(word_start) = start {
        words.push(&text[word_start..]);
    }

    words
}

fn brace_character_spans(span: Span, source: &str) -> Vec<Span> {
    let text = span.slice(source);
    text.char_indices()
        .filter(|&(_, ch)| matches!(ch, '{' | '}'))
        .filter_map(|(offset, _)| {
            let absolute_offset = span.start.offset + offset;
            if has_odd_backslash_run_before(source, absolute_offset) {
                return None;
            }
            let position = span.start.advanced_by(&text[..offset]);
            Some(Span::from_positions(position, position))
        })
        .collect()
}

fn brace_span_has_escaped_dollar_prefix(span: Span, source: &str) -> bool {
    let span_text = span.slice(source);
    if span_text.starts_with("${") {
        return has_odd_backslash_run_before(source, span.start.offset);
    }

    has_escaped_dollar_before(source, span.start.offset)
}

fn brace_syntax_with_whitespace_is_literal(brace: shuck_ast::BraceSyntax, source: &str) -> bool {
    if !matches!(brace.kind, BraceSyntaxKind::Expansion(_)) {
        return false;
    }

    #[derive(Clone, Copy)]
    enum QuoteState {
        Single,
        Double,
    }

    let text = brace.span.slice(source);
    let mut index = 0usize;
    let mut quote_state = None;

    while index < text.len() {
        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        let ch_len = ch.len_utf8();

        if let Some(state) = quote_state {
            match state {
                QuoteState::Single => {
                    if ch == '\'' {
                        quote_state = None;
                    }
                    index += ch_len;
                    continue;
                }
                QuoteState::Double => {
                    if ch == '\\' {
                        index += ch_len;
                        if let Some(escaped) = text[index..].chars().next() {
                            index += escaped.len_utf8();
                        }
                        continue;
                    }
                    if ch == '"' {
                        quote_state = None;
                    }
                    index += ch_len;
                    continue;
                }
            }
        }

        if ch == '\\' {
            index += ch_len;
            if text[index..].starts_with("\r\n") {
                index += "\r\n".len();
                continue;
            }
            if text[index..].starts_with('\n') {
                index += '\n'.len_utf8();
                continue;
            }
            if let Some(escaped) = text[index..].chars().next() {
                index += escaped.len_utf8();
            }
            continue;
        }

        if ch == '\'' {
            quote_state = Some(QuoteState::Single);
            index += ch_len;
            continue;
        }

        if ch == '"' {
            quote_state = Some(QuoteState::Double);
            index += ch_len;
            continue;
        }

        if ch.is_whitespace() {
            return true;
        }

        index += ch_len;
    }

    false
}

fn word_is_empty_brace_pair_variant(word: &Word, source: &str) -> bool {
    matches!(word.span.slice(source), "{}" | "\\{\\}")
}

fn unclassified_literal_brace_spans(word: &Word, source: &str) -> Vec<Span> {
    let span = word.span;
    let text = span.slice(source);
    let mut excluded = Vec::new();
    collect_dynamic_brace_exclusions(
        &word.parts,
        span.start.offset,
        span.end.offset,
        source,
        &mut excluded,
    );
    excluded.extend(
        word.brace_syntax()
            .iter()
            .map(|brace| DynamicBraceExcludedSpan {
                start_offset: brace.span.start.offset - span.start.offset,
                end_offset: brace.span.end.offset - span.start.offset,
                kind: DynamicBraceExcludedSpanKind::RuntimeShellSyntax,
            }),
    );
    excluded.sort_by_key(|span| (span.start_offset, span.end_offset));

    let mut spans = Vec::new();
    let mut excluded_index = 0usize;
    let mut index = 0usize;
    let mut unmatched_opens = Vec::new();

    while index < text.len() {
        while let Some(excluded_span) = excluded.get(excluded_index).copied() {
            if excluded_span.end_offset <= index {
                excluded_index += 1;
                continue;
            }
            if excluded_span.start_offset > index {
                break;
            }

            index = excluded_span.end_offset;
            excluded_index += 1;
        }

        if index >= text.len() {
            break;
        }

        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        let ch_len = ch.len_utf8();

        if text[index..].starts_with("\\${")
            && let Some(end_offset) =
                find_runtime_parameter_closing_brace(text, index + '\\'.len_utf8())
        {
            index = end_offset;
            continue;
        }

        if ch == '\\' {
            index += ch_len;
            if let Some(escaped) = text[index..].chars().next() {
                index += escaped.len_utf8();
            }
            continue;
        }

        if ch == '{' {
            unmatched_opens.push(index);
        } else if ch == '}' && unmatched_opens.pop().is_none() {
            let position = span.start.advanced_by(&text[..index]);
            spans.push(Span::from_positions(position, position));
        }

        index += ch_len;
    }

    spans.extend(unmatched_opens.into_iter().map(|offset| {
        let position = span.start.advanced_by(&text[..offset]);
        Span::from_positions(position, position)
    }));

    spans
}

fn uncovered_command_brace_spans(
    commands: &[CommandFact<'_>],
    source: &str,
    heredoc_ranges: &[TextRange],
) -> Vec<Span> {
    let mut spans = Vec::new();

    for command in commands {
        let Command::Simple(simple) = command.command() else {
            continue;
        };
        let command_span = command.span();
        let mut covered = Vec::new();

        if !simple.name.span.slice(source).is_empty() {
            covered.push(simple.name.span);
        }
        covered.extend(simple.args.iter().map(|word| word.span));
        covered.extend(simple.assignments.iter().map(|assignment| assignment.span));
        covered.extend(command.redirects().iter().map(|redirect| redirect.span));
        covered.extend(command.substitution_facts().iter().map(|fact| fact.span()));
        covered.extend(
            command
                .redirects()
                .iter()
                .filter_map(|redirect| redirect.fd_var_span),
        );
        covered.extend(
            command
                .redirects()
                .iter()
                .filter_map(|redirect| redirect_fd_var_brace_span(redirect, source)),
        );
        covered.extend(
            command
                .redirects()
                .iter()
                .filter_map(|redirect| redirect.heredoc().map(|heredoc| heredoc.body.span)),
        );
        covered.extend(
            command
                .redirects()
                .iter()
                .filter_map(|redirect| redirect.fd_var_span),
        );

        if covered.is_empty() {
            continue;
        }

        covered.sort_by_key(|span| (span.start.offset, span.end.offset));

        let mut cursor = command_span.start.offset;
        for span in covered {
            if span.start.offset > cursor {
                spans.extend(raw_literal_brace_spans(
                    command_span,
                    cursor,
                    span.start.offset,
                    source,
                    RawLiteralBraceScanMode::All,
                    heredoc_ranges,
                ));
            }
            cursor = cursor.max(span.end.offset);
        }

        if command_span.end.offset > cursor {
            spans.extend(raw_literal_brace_spans(
                command_span,
                cursor,
                command_span.end.offset,
                source,
                RawLiteralBraceScanMode::All,
                heredoc_ranges,
            ));
        }
    }

    spans
}

fn redirect_fd_var_brace_span(redirect: &Redirect, source: &str) -> Option<Span> {
    let fd_var_span = redirect.fd_var_span?;
    let start_offset = fd_var_span.start.offset.checked_sub('{'.len_utf8())?;
    let end_offset = fd_var_span.end.offset.checked_add('}'.len_utf8())?;
    if source.get(start_offset..fd_var_span.start.offset)? != "{" {
        return None;
    }
    if source.get(fd_var_span.end.offset..end_offset)? != "}" {
        return None;
    }

    Some(Span::from_positions(
        Position {
            line: fd_var_span.start.line,
            column: fd_var_span.start.column.checked_sub(1)?,
            offset: start_offset,
        },
        Position {
            line: fd_var_span.end.line,
            column: fd_var_span.end.column + 1,
            offset: end_offset,
        },
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawLiteralBraceScanMode {
    All,
    UnmatchedOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawLiteralBraceQuoteState {
    Single,
    Double,
}

fn raw_literal_brace_spans(
    container_span: Span,
    scan_start: usize,
    scan_end: usize,
    source: &str,
    mode: RawLiteralBraceScanMode,
    excluded_ranges: &[TextRange],
) -> Vec<Span> {
    let mut relevant_excluded = excluded_ranges
        .iter()
        .filter_map(|range| {
            let start = usize::from(range.start());
            let end = usize::from(range.end());
            if end <= scan_start || start >= scan_end {
                return None;
            }
            Some((start.max(scan_start), end.min(scan_end)))
        })
        .collect::<Vec<_>>();
    relevant_excluded.sort_unstable_by_key(|&(start, end)| (start, end));

    let mut spans = Vec::new();
    let mut unmatched_opens = Vec::new();
    let mut cursor = scan_start;
    for (start, end) in relevant_excluded {
        if start > cursor {
            spans.extend(raw_literal_brace_spans_without_exclusions(
                container_span,
                cursor,
                start,
                source,
                mode,
                &mut unmatched_opens,
            ));
        }
        cursor = cursor.max(end);
    }

    if scan_end > cursor {
        spans.extend(raw_literal_brace_spans_without_exclusions(
            container_span,
            cursor,
            scan_end,
            source,
            mode,
            &mut unmatched_opens,
        ));
    }

    if mode == RawLiteralBraceScanMode::UnmatchedOnly {
        spans.extend(unmatched_opens);
    }

    spans
}

fn raw_literal_brace_spans_without_exclusions(
    _container_span: Span,
    scan_start: usize,
    scan_end: usize,
    source: &str,
    mode: RawLiteralBraceScanMode,
    unmatched_opens: &mut Vec<Span>,
) -> Vec<Span> {
    let Some(text) = source.get(scan_start..scan_end) else {
        return Vec::new();
    };
    if text.is_empty() {
        return Vec::new();
    }

    let mut spans = Vec::new();
    let mut index = 0usize;
    let mut quote_state = None;
    let mut in_comment = false;

    while index < text.len() {
        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        let ch_len = ch.len_utf8();

        if in_comment {
            if ch == '\n' {
                in_comment = false;
            }
            index += ch_len;
            continue;
        }

        if let Some(state) = quote_state {
            match state {
                RawLiteralBraceQuoteState::Single => {
                    if ch == '\'' {
                        quote_state = None;
                    }
                    index += ch_len;
                    continue;
                }
                RawLiteralBraceQuoteState::Double => {
                    if ch == '\\' {
                        index += ch_len;
                        if let Some(escaped) = text[index..].chars().next() {
                            index += escaped.len_utf8();
                        }
                        continue;
                    }
                    if ch == '"' {
                        quote_state = None;
                    }
                    index += ch_len;
                    continue;
                }
            }
        }

        if text[index..].starts_with("${")
            && let Some(end_offset) = find_runtime_parameter_closing_brace(text, index)
        {
            index = end_offset;
            continue;
        }

        if ch == '\\' {
            index += ch_len;
            if let Some(escaped) = text[index..].chars().next() {
                index += escaped.len_utf8();
            }
            continue;
        }

        if ch == '#' {
            in_comment = true;
            index += ch_len;
            continue;
        }

        if ch == '\'' {
            quote_state = Some(RawLiteralBraceQuoteState::Single);
            index += ch_len;
            continue;
        }

        if ch == '"' {
            quote_state = Some(RawLiteralBraceQuoteState::Double);
            index += ch_len;
            continue;
        }

        if matches!(ch, '{' | '}') {
            if mode == RawLiteralBraceScanMode::UnmatchedOnly
                && brace_at_command_start(text, index, ch)
            {
                index += ch_len;
                continue;
            }

            let Some(position) = position_at_offset(source, scan_start + index) else {
                index += ch_len;
                continue;
            };
            let span = Span::from_positions(position, position);
            match mode {
                RawLiteralBraceScanMode::All => spans.push(span),
                RawLiteralBraceScanMode::UnmatchedOnly => {
                    if ch == '{' {
                        unmatched_opens.push(span);
                    } else if unmatched_opens.pop().is_none() {
                        spans.push(span);
                    }
                }
            }
        }

        index += ch_len;
    }

    spans
}

fn brace_at_command_start(text: &str, index: usize, ch: char) -> bool {
    match ch {
        '{' => opening_brace_starts_shell_group(text, index),
        '}' => closing_brace_ends_shell_group(text, index),
        _ => false,
    }
}

fn literal_brace_syntax_looks_like_active_expansion(
    brace: shuck_ast::BraceSyntax,
    source: &str,
) -> bool {
    if !matches!(brace.kind, BraceSyntaxKind::Literal) {
        return false;
    }

    let text = brace.span.slice(source);
    brace_text_has_unescaped_comma_or_sequence(text) && !text.chars().any(char::is_whitespace)
}

fn brace_text_has_unescaped_comma_or_sequence(text: &str) -> bool {
    let Some(inner) = text
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
    else {
        return false;
    };

    let mut chars = inner.chars().peekable();
    let mut previous = None;
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            chars.next();
            previous = None;
            continue;
        }

        if ch == ',' {
            return true;
        }
        if ch == '.' && previous == Some('.') {
            return true;
        }

        previous = Some(ch);
    }

    false
}

fn opening_brace_starts_shell_group(text: &str, index: usize) -> bool {
    let Some(next) = text[index + '{'.len_utf8()..].chars().next() else {
        return false;
    };
    if !next.is_whitespace() {
        return false;
    }

    let prefix = text[..index].trim_end_matches([' ', '\t']);
    let Some(last) = prefix.chars().next_back() else {
        return true;
    };

    match last {
        '\n' | '&' | '|' | '(' | ')' => true,
        ';' => prefix.chars().rev().nth(1) != Some('\\'),
        'o' => prefix.ends_with("do"),
        'n' => prefix.ends_with("then"),
        'e' => prefix.ends_with("else"),
        'f' => prefix.ends_with("elif"),
        _ => false,
    }
}

fn closing_brace_ends_shell_group(text: &str, index: usize) -> bool {
    let prefix = text[..index].trim_end_matches([' ', '\t']);
    let Some(last) = prefix.chars().next_back() else {
        return true;
    };

    match last {
        '\n' | '&' | '|' | '(' => true,
        ';' => prefix.chars().rev().nth(1) != Some('\\'),
        _ => false,
    }
}

fn unmatched_command_substitution_brace_spans(
    commands: &[CommandFact<'_>],
    source: &str,
    heredoc_ranges: &[TextRange],
) -> Vec<Span> {
    let mut spans = Vec::new();

    for substitution in commands
        .iter()
        .flat_map(|command| command.substitution_facts())
    {
        let Some((container_span, body_start, body_end)) =
            command_substitution_body_offsets(substitution.span(), source)
        else {
            continue;
        };

        if body_end > body_start {
            spans.extend(raw_literal_brace_spans(
                container_span,
                body_start,
                body_end,
                source,
                RawLiteralBraceScanMode::UnmatchedOnly,
                heredoc_ranges,
            ));
        }
    }

    spans
}

fn command_substitution_body_offsets(span: Span, source: &str) -> Option<(Span, usize, usize)> {
    let text = span.slice(source);
    if text.starts_with("$(") && text.ends_with(')') && text.len() >= 3 {
        return Some((
            span,
            span.start.offset + "$(".len(),
            span.end.offset - ')'.len_utf8(),
        ));
    }
    if text.starts_with('`') && text.ends_with('`') && text.len() >= 2 {
        return Some((
            span,
            span.start.offset + '`'.len_utf8(),
            span.end.offset - '`'.len_utf8(),
        ));
    }
    None
}

#[derive(Debug, Clone, Copy)]
struct LiteralBraceCandidate {
    open_offset: usize,
    after_escaped_dollar: bool,
    has_excluded_content_inside: bool,
    has_nested_parameter_inside: bool,
    has_runtime_shell_sigil_inside: bool,
    has_brace_expansion_delimiter: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DynamicBraceExcludedSpanKind {
    Quoted,
    RuntimeShellSyntax,
}

#[derive(Debug, Clone, Copy)]
struct DynamicBraceExcludedSpan {
    start_offset: usize,
    end_offset: usize,
    kind: DynamicBraceExcludedSpanKind,
}

fn escaped_parameter_expansion_brace_edge_spans(word: &Word, source: &str) -> Vec<Span> {
    let span = word.span;
    let text = span.slice(source);
    let mut spans = Vec::new();
    let mut literal_stack: Vec<LiteralBraceCandidate> = Vec::new();
    let mut excluded = Vec::new();
    collect_dynamic_brace_exclusions(
        &word.parts,
        span.start.offset,
        span.end.offset,
        source,
        &mut excluded,
    );
    excluded.sort_by_key(|span| (span.start_offset, span.end_offset));
    let mut excluded_index = 0usize;
    let mut index = 0usize;
    let mut previous_char = None;
    let mut previous_char_escaped = false;

    while index < text.len() {
        while let Some(excluded_span) = excluded.get(excluded_index).copied() {
            if excluded_span.end_offset <= index {
                excluded_index += 1;
                continue;
            }

            if excluded_span.start_offset > index {
                break;
            }

            if excluded_span.kind == DynamicBraceExcludedSpanKind::RuntimeShellSyntax
                && let Some(current) = literal_stack.last_mut()
            {
                current.has_runtime_shell_sigil_inside = true;
            }
            if let Some(current) = literal_stack.last_mut() {
                current.has_excluded_content_inside = true;
            }
            if excluded_span.kind == DynamicBraceExcludedSpanKind::RuntimeShellSyntax
                && excluded_runtime_syntax_has_escaped_dollar_prefix(
                    text,
                    excluded_span.start_offset,
                    excluded_span.end_offset,
                )
            {
                let excluded_text = &text[excluded_span.start_offset..excluded_span.end_offset];
                let open_offset = if excluded_text.starts_with("${") {
                    Some(excluded_span.start_offset + '$'.len_utf8())
                } else if excluded_text.starts_with('{') {
                    Some(excluded_span.start_offset)
                } else {
                    None
                };
                if let Some(open_offset) = open_offset
                    && excluded_text.ends_with('}')
                    && excluded_span.end_offset > open_offset + 1
                {
                    let open = span.start.advanced_by(&text[..open_offset]);
                    let close = span
                        .start
                        .advanced_by(&text[..excluded_span.end_offset - '}'.len_utf8()]);
                    spans.push(Span::from_positions(open, open));
                    spans.push(Span::from_positions(close, close));
                }
            }
            previous_char = None;
            previous_char_escaped = false;
            index = excluded_span.end_offset;
            excluded_index += 1;
        }

        if index >= text.len() {
            break;
        }

        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        let ch_len = ch.len_utf8();

        if ch == '\\' {
            index += ch_len;
            if let Some(escaped) = text[index..].chars().next() {
                previous_char = Some(escaped);
                previous_char_escaped = true;
                index += escaped.len_utf8();
            } else {
                previous_char = Some('\\');
                previous_char_escaped = false;
            }
            continue;
        }

        if ch == '{' {
            if previous_char == Some('$')
                && !previous_char_escaped
                && let Some(candidate) = literal_stack.last_mut()
            {
                candidate.has_nested_parameter_inside = true;
            }
            literal_stack.push(LiteralBraceCandidate {
                open_offset: index,
                after_escaped_dollar: previous_char == Some('$') && previous_char_escaped,
                has_excluded_content_inside: false,
                has_nested_parameter_inside: false,
                has_runtime_shell_sigil_inside: false,
                has_brace_expansion_delimiter: false,
            });
        } else if ch == ','
            && let Some(candidate) = literal_stack.last_mut()
        {
            candidate.has_brace_expansion_delimiter = true;
        } else if ch == '.'
            && previous_char == Some('.')
            && !previous_char_escaped
            && let Some(candidate) = literal_stack.last_mut()
        {
            candidate.has_brace_expansion_delimiter = true;
        } else if ch == '}'
            && let Some(candidate) = literal_stack.pop()
            && index > candidate.open_offset + 1
            && (candidate.after_escaped_dollar
                || candidate.has_excluded_content_inside
                || candidate.has_runtime_shell_sigil_inside)
            && !(candidate.after_escaped_dollar && candidate.has_nested_parameter_inside)
            && !candidate.has_brace_expansion_delimiter
            && !brace_pair_matches_nonliteral_syntax(word, candidate.open_offset, index)
        {
            let open = span.start.advanced_by(&text[..candidate.open_offset]);
            let close = span.start.advanced_by(&text[..index]);
            spans.push(Span::from_positions(open, open));
            spans.push(Span::from_positions(close, close));
        }

        previous_char = Some(ch);
        previous_char_escaped = false;
        index += ch_len;
    }

    spans.extend(raw_escaped_parameter_brace_edge_spans(word, source));
    spans
}

fn excluded_runtime_syntax_has_escaped_dollar_prefix(
    text: &str,
    start_offset: usize,
    end_offset: usize,
) -> bool {
    let start_offset = start_offset.min(text.len());
    let end_offset = end_offset.min(text.len());
    if start_offset >= end_offset {
        return false;
    }

    let excluded_text = &text[start_offset..end_offset];
    if excluded_text.starts_with("${") {
        return has_odd_backslash_run_before(text, start_offset);
    }
    if excluded_text.starts_with('{') {
        return has_escaped_dollar_before(text, start_offset);
    }
    false
}

fn has_odd_backslash_run_before(text: &str, offset: usize) -> bool {
    let offset = offset.min(text.len());
    text[..offset]
        .chars()
        .rev()
        .take_while(|&ch| ch == '\\')
        .count()
        % 2
        == 1
}

fn has_escaped_dollar_before(text: &str, offset: usize) -> bool {
    let offset = offset.min(text.len());
    let prefix = &text[..offset];
    let Some((dollar_offset, '$')) = prefix.char_indices().next_back() else {
        return false;
    };

    has_odd_backslash_run_before(text, dollar_offset)
}

fn collect_dynamic_brace_exclusions(
    parts: &[WordPartNode],
    word_base_offset: usize,
    word_end_offset: usize,
    source: &str,
    out: &mut Vec<DynamicBraceExcludedSpan>,
) {
    for part in parts {
        match &part.kind {
            WordPart::Literal(_) => {}
            WordPart::DoubleQuoted { .. } if !part.span.slice(source).starts_with("\\\"") => {
                out.push(DynamicBraceExcludedSpan {
                    start_offset: part.span.start.offset - word_base_offset,
                    end_offset: part.span.end.offset - word_base_offset,
                    kind: DynamicBraceExcludedSpanKind::Quoted,
                });
            }
            WordPart::DoubleQuoted { parts, .. } => {
                collect_dynamic_brace_exclusions(
                    parts,
                    word_base_offset,
                    word_end_offset,
                    source,
                    out,
                );
            }
            WordPart::SingleQuoted { .. } => {
                out.push(DynamicBraceExcludedSpan {
                    start_offset: part.span.start.offset - word_base_offset,
                    end_offset: part.span.end.offset - word_base_offset,
                    kind: DynamicBraceExcludedSpanKind::Quoted,
                });
            }
            WordPart::CommandSubstitution { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::Variable(_)
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::Substring { .. }
            | WordPart::ArraySlice { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::Transformation { .. }
            | WordPart::ZshQualifiedGlob(_) => out.push(runtime_shell_dynamic_brace_exclusion(
                part,
                word_base_offset,
                word_end_offset,
                source,
            )),
        }
    }
}

fn runtime_shell_dynamic_brace_exclusion(
    part: &WordPartNode,
    word_base_offset: usize,
    word_end_offset: usize,
    source: &str,
) -> DynamicBraceExcludedSpan {
    let start_offset = part.span.start.offset - word_base_offset;
    let mut end_offset = part.span.end.offset - word_base_offset;
    let part_text = part.span.slice(source);
    let word_text = &source[word_base_offset..word_end_offset.min(source.len())];

    if let Some(relative_parameter_start) = part_text.find("${") {
        end_offset = find_runtime_parameter_closing_brace(
            word_text,
            start_offset + relative_parameter_start,
        )
        .map_or(end_offset, |closing_offset| end_offset.max(closing_offset));
    }

    DynamicBraceExcludedSpan {
        start_offset,
        end_offset,
        kind: DynamicBraceExcludedSpanKind::RuntimeShellSyntax,
    }
}

fn find_runtime_parameter_closing_brace(text: &str, start_offset: usize) -> Option<usize> {
    if start_offset >= text.len() || !text[start_offset..].starts_with("${") {
        return None;
    }

    let bytes = text.as_bytes();
    let mut index = start_offset + "${".len();
    let mut depth = 1usize;

    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = advance_escaped_char_boundary(text, index);
            continue;
        }

        if index + 2 < bytes.len()
            && is_unescaped_dollar(bytes, index)
            && bytes[index + 1] == b'('
            && bytes[index + 2] == b'('
        {
            index = find_wrapped_arithmetic_end(bytes, index)?;
            continue;
        }

        if index + 1 < bytes.len() && is_unescaped_dollar(bytes, index) && bytes[index + 1] == b'('
        {
            index = find_command_substitution_end(bytes, index)?;
            continue;
        }

        if index + 1 < bytes.len() && is_unescaped_dollar(bytes, index) && bytes[index + 1] == b'{'
        {
            depth += 1;
            index += "${".len();
            continue;
        }

        match bytes[index] {
            b'\'' => index = skip_single_quoted(bytes, index + 1)?,
            b'"' => index = skip_double_quoted(bytes, index + 1)?,
            b'`' => index = skip_backticks(bytes, index + 1)?,
            b'}' => {
                depth -= 1;
                index += '}'.len_utf8();
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {
                index += text[index..].chars().next()?.len_utf8();
            }
        }
    }

    None
}

fn raw_escaped_parameter_brace_edge_spans(word: &Word, source: &str) -> Vec<Span> {
    let span = word.span;
    let text = span.slice(source);
    let mut excluded = Vec::new();
    collect_raw_escaped_parameter_exclusions(&word.parts, span.start.offset, source, &mut excluded);
    excluded.sort_by_key(|span| (span.start_offset, span.end_offset));

    let mut spans = Vec::new();
    let mut excluded_index = 0usize;
    let mut index = 0usize;
    let mut previous_char = None;
    let mut previous_char_escaped = false;
    let mut escaped_parameter_stack: Vec<(usize, bool)> = Vec::new();
    let mut parameter_depth = 0usize;

    while index < text.len() {
        while let Some(excluded_span) = excluded.get(excluded_index).copied() {
            if excluded_span.end_offset <= index {
                excluded_index += 1;
                continue;
            }
            if excluded_span.start_offset > index {
                break;
            }

            previous_char = None;
            previous_char_escaped = false;
            index = excluded_span.end_offset;
            excluded_index += 1;
        }

        if index >= text.len() {
            break;
        }

        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        let ch_len = ch.len_utf8();

        if ch == '\\' {
            index += ch_len;
            if let Some(escaped) = text[index..].chars().next() {
                previous_char = Some(escaped);
                previous_char_escaped = true;
                index += escaped.len_utf8();
            } else {
                previous_char = Some('\\');
                previous_char_escaped = false;
            }
            continue;
        }

        if ch == '{' {
            if previous_char == Some('$') && previous_char_escaped {
                escaped_parameter_stack.push((index, false));
            } else if previous_char == Some('$') && !previous_char_escaped {
                if let Some((_, has_nested_parameter_inside)) = escaped_parameter_stack.last_mut() {
                    *has_nested_parameter_inside = true;
                }
                parameter_depth += 1;
            }
        } else if ch == '}' {
            if parameter_depth > 0 {
                parameter_depth -= 1;
            } else if let Some((open_offset, has_nested_parameter_inside)) =
                escaped_parameter_stack.pop()
                && !has_nested_parameter_inside
                && !brace_pair_matches_nonliteral_syntax(word, open_offset, index)
            {
                let open = span.start.advanced_by(&text[..open_offset]);
                let close = span.start.advanced_by(&text[..index]);
                spans.push(Span::from_positions(open, open));
                spans.push(Span::from_positions(close, close));
            }
        }

        previous_char = Some(ch);
        previous_char_escaped = false;
        index += ch_len;
    }

    spans
}

fn brace_pair_matches_nonliteral_syntax(
    word: &Word,
    open_offset: usize,
    close_offset: usize,
) -> bool {
    let absolute_open_offset = word.span.start.offset + open_offset;
    let absolute_close_offset = word.span.start.offset + close_offset + '}'.len_utf8();

    word.brace_syntax().iter().any(|brace| {
        brace.kind != BraceSyntaxKind::Literal
            && brace.span.start.offset == absolute_open_offset
            && brace.span.end.offset == absolute_close_offset
    })
}

fn span_inside_nested_escaped_parameter_template(word: &Word, span: Span, source: &str) -> bool {
    if span.start.offset < word.span.start.offset || span.start.offset >= word.span.end.offset {
        return false;
    }

    let text = word.span.slice(source);
    let relative_offset = span.start.offset - word.span.start.offset;
    let mut index = 0usize;

    while index < text.len() {
        if text[index..].starts_with("\\${")
            && let Some(end_offset) =
                find_runtime_parameter_closing_brace(text, index + '\\'.len_utf8())
        {
            let body_start = index + "\\${".len();
            let body_end = end_offset.saturating_sub('}'.len_utf8());
            let has_nested_parameter =
                body_start < body_end && text[body_start..body_end].contains("${");
            let open_brace_offset = index + "\\$".len();
            if has_nested_parameter
                && relative_offset > open_brace_offset
                && relative_offset < end_offset.saturating_sub('}'.len_utf8())
            {
                return true;
            }
            index = end_offset;
            continue;
        }

        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        let ch_len = ch.len_utf8();

        if ch == '\\' {
            index += ch_len;
            if let Some(escaped) = text[index..].chars().next() {
                index += escaped.len_utf8();
            }
            continue;
        }

        index += ch_len;
    }

    false
}

fn collect_raw_escaped_parameter_exclusions(
    parts: &[WordPartNode],
    word_base_offset: usize,
    source: &str,
    out: &mut Vec<DynamicBraceExcludedSpan>,
) {
    for part in parts {
        match &part.kind {
            WordPart::Literal(_)
            | WordPart::Variable(_)
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::Substring { .. }
            | WordPart::ArraySlice { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::Transformation { .. }
            | WordPart::ZshQualifiedGlob(_) => {}
            WordPart::DoubleQuoted { .. } if !part.span.slice(source).starts_with("\\\"") => {
                out.push(DynamicBraceExcludedSpan {
                    start_offset: part.span.start.offset - word_base_offset,
                    end_offset: part.span.end.offset - word_base_offset,
                    kind: DynamicBraceExcludedSpanKind::Quoted,
                });
            }
            WordPart::DoubleQuoted { .. } => {}
            WordPart::SingleQuoted { .. }
            | WordPart::CommandSubstitution { .. }
            | WordPart::ProcessSubstitution { .. } => out.push(DynamicBraceExcludedSpan {
                start_offset: part.span.start.offset - word_base_offset,
                end_offset: part.span.end.offset - word_base_offset,
                kind: DynamicBraceExcludedSpanKind::Quoted,
            }),
        }
    }
}

fn is_inline_shellcheck_directive(comment_text: &str) -> bool {
    let body = comment_text
        .trim_start()
        .trim_start_matches('#')
        .trim_start();
    let Some(remainder) = strip_prefix_ignore_ascii_case(body, "shellcheck") else {
        return false;
    };
    let Some(first) = remainder.chars().next() else {
        return false;
    };
    if !first.is_ascii_whitespace() {
        return false;
    }

    let mut body = remainder;
    if let Some((before, _)) = body.split_once('#') {
        body = before;
    }

    body.split_ascii_whitespace().any(|part| {
        [
            "disable=",
            "enable=",
            "disable-file=",
            "source=",
            "shell=",
            "external-sources=",
        ]
        .into_iter()
        .any(|prefix| {
            strip_prefix_ignore_ascii_case(part, prefix)
                .is_some_and(|value| !value.trim().is_empty())
        })
    })
}

fn strip_prefix_ignore_ascii_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let candidate = text.get(..prefix.len())?;
    candidate
        .eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}


