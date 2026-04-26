use shuck_ast::{
    ArithmeticExpr, BourneParameterExpansion, CaseItem, CommandSubstitutionSyntax, ConditionalExpr,
    ParameterExpansion, ParameterExpansionSyntax, ParameterOp, Pattern, PatternGroupKind,
    PatternPart, Position, PrefixMatchKind, Span, SubscriptSelector, VarRef, Word, WordPart,
    WordPartNode, ZshExpansionTarget,
};

use super::BacktickEscapedParameter;

pub fn command_substitution_part_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_command_substitution_spans(&word.parts, &mut spans);
    spans
}

pub fn command_substitution_part_spans_in_source(word: &Word, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_command_substitution_part_spans_in_source(word, source, &mut spans);
    spans
}

pub fn collect_command_substitution_part_spans_in_source(
    word: &Word,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_command_substitution_spans(&word.parts, spans);
    normalize_command_substitution_spans(spans, source);
}

pub fn arithmetic_expansion_part_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_arithmetic_expansion_spans(&word.parts, &mut spans);
    spans
}

pub fn parenthesized_arithmetic_expansion_part_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_parenthesized_arithmetic_expansion_spans(&word.parts, &mut spans);
    spans
}

pub fn unquoted_command_substitution_part_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_unquoted_command_substitution_spans(&word.parts, false, &mut spans);
    spans
}

pub fn unquoted_command_substitution_part_spans_in_source(word: &Word, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_unquoted_command_substitution_part_spans_in_source(word, source, &mut spans);
    spans
}

pub fn collect_unquoted_command_substitution_part_spans_in_source(
    word: &Word,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_unquoted_command_substitution_spans(&word.parts, false, spans);
    normalize_command_substitution_spans(spans, source);
}

pub fn unquoted_dollar_paren_command_substitution_part_spans_in_source(
    word: &Word,
    source: &str,
) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_unquoted_dollar_paren_command_substitution_part_spans_in_source(
        word, source, &mut spans,
    );
    spans
}

pub fn collect_unquoted_dollar_paren_command_substitution_part_spans_in_source(
    word: &Word,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_unquoted_dollar_paren_command_substitution_spans(&word.parts, false, spans);
    normalize_command_substitution_spans(spans, source);
}

pub fn unescaped_backtick_command_substitution_span(span: Span, source: &str) -> Option<Span> {
    let normalized = normalize_command_substitution_span(span, source);
    let text = normalized.slice(source);
    if !text.starts_with('`') || !text.ends_with('`') || span_is_escaped(normalized, source) {
        return None;
    }

    Some(normalized)
}

pub(crate) fn shellcheck_collapsed_backtick_part_span(
    span: Span,
    source: &str,
    backtick_spans: &[Span],
) -> Span {
    let deescaped =
        shellcheck_deescaped_backtick_part_span(span, source, backtick_spans).unwrap_or(span);
    collapse_backtick_continuation_span(deescaped, source, backtick_spans).unwrap_or(deescaped)
}

pub(crate) fn shellcheck_collapsed_backtick_part_span_in_source(span: Span, source: &str) -> Span {
    let backtick_spans = backtick_substitution_spans(source);
    shellcheck_collapsed_backtick_part_span(span, source, &backtick_spans)
}

pub fn array_expansion_part_spans(word: &Word, _source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_array_expansion_part_spans(word, &mut spans);
    spans
}

pub fn collect_array_expansion_part_spans(word: &Word, spans: &mut Vec<Span>) {
    collect_array_expansion_spans(&word.parts, false, false, spans);
}

pub fn all_elements_array_expansion_part_spans(word: &Word, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_all_elements_array_expansion_part_spans(word, source, &mut spans);
    spans
}

pub fn collect_all_elements_array_expansion_part_spans(
    word: &Word,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_all_elements_array_expansion_spans(&word.parts, source, spans);
}

pub fn word_has_all_elements_array_expansion_syntax(word: &Word) -> bool {
    parts_have_all_elements_array_expansion_syntax(&word.parts)
}

pub fn direct_all_elements_array_expansion_part_spans(word: &Word, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_direct_all_elements_array_expansion_part_spans(word, source, &mut spans);
    spans
}

pub fn collect_direct_all_elements_array_expansion_part_spans(
    word: &Word,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_direct_all_elements_array_expansion_spans(&word.parts, source, spans);
}

pub fn unquoted_all_elements_array_expansion_part_spans(word: &Word, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_unquoted_all_elements_array_expansion_part_spans(word, source, &mut spans);
    spans
}

pub fn collect_unquoted_all_elements_array_expansion_part_spans(
    word: &Word,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_unquoted_all_elements_array_expansion_spans(&word.parts, false, source, spans);
}

pub fn word_all_elements_array_slice_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_all_elements_array_slice_spans(&word.parts, false, false, &mut spans);
    spans
}

pub fn word_quoted_all_elements_array_slice_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_all_elements_array_slice_spans(&word.parts, false, true, &mut spans);
    spans
}

pub fn word_has_quoted_all_elements_array_slice(word: &Word) -> bool {
    !word_quoted_all_elements_array_slice_spans(word).is_empty()
}

pub fn word_has_direct_all_elements_array_expansion_in_source(word: &Word, source: &str) -> bool {
    !direct_all_elements_array_expansion_part_spans(word, source).is_empty()
}

pub fn word_all_elements_array_slice_span_in_source(word: &Word, source: &str) -> Option<Span> {
    word_all_elements_array_slice_spans(word)
        .into_iter()
        .find(|span| !span_is_escaped(*span, source))
}

pub fn word_quoted_unindexed_bash_source_span_in_source(word: &Word, source: &str) -> Option<Span> {
    let mut spans = Vec::new();
    collect_quoted_unindexed_bash_source_spans(&word.parts, false, source, &mut spans);
    spans.into_iter().next()
}

pub fn unquoted_array_expansion_part_spans(word: &Word, _source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_unquoted_array_expansion_part_spans(word, &mut spans);
    spans
}

pub fn collect_unquoted_array_expansion_part_spans(word: &Word, spans: &mut Vec<Span>) {
    collect_array_expansion_spans(&word.parts, false, true, spans);
}

pub fn expansion_part_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_expansion_spans(&word.parts, &mut spans);
    spans
}

pub fn active_expansion_spans_in_source(word: &Word, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_active_expansion_spans_in_source(word, source, &mut spans);
    spans
}

pub fn collect_active_expansion_spans_in_source(word: &Word, source: &str, spans: &mut Vec<Span>) {
    collect_expansion_spans(&word.parts, spans);
    normalize_command_substitution_spans(spans, source);
    spans.extend(
        word.brace_syntax()
            .iter()
            .copied()
            .filter(|brace| brace.expands())
            .map(|brace| brace.span),
    );
    spans.sort_unstable_by_key(|span| (span.start.offset, span.end.offset));
    spans.dedup();
}

pub fn scalar_expansion_part_spans(word: &Word, _source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_scalar_expansion_part_spans(word, &mut spans);
    spans
}

pub fn collect_scalar_expansion_part_spans(word: &Word, spans: &mut Vec<Span>) {
    collect_scalar_expansion_spans(&word.parts, false, false, spans);
}

pub fn unquoted_scalar_expansion_part_spans(word: &Word, _source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_unquoted_scalar_expansion_part_spans(word, &mut spans);
    spans
}

pub fn collect_unquoted_scalar_expansion_part_spans(word: &Word, spans: &mut Vec<Span>) {
    collect_scalar_expansion_spans(&word.parts, false, true, spans);
}

pub fn double_quoted_scalar_affix_span(word: &Word) -> Option<Span> {
    if !word.is_fully_double_quoted() {
        return None;
    }

    let mut saw_literal = false;
    let mut saw_scalar_expansion = false;
    let mut literal_span = None;
    if !collect_double_quoted_scalar_affix_state(
        &word.parts,
        &mut saw_literal,
        &mut saw_scalar_expansion,
        &mut literal_span,
    ) {
        return None;
    }

    (saw_literal && saw_scalar_expansion)
        .then_some(literal_span)
        .flatten()
}

pub fn word_shell_quoting_literal_span(word: &Word, source: &str) -> Option<Span> {
    let mut excluded = Vec::new();
    collect_literal_scan_exclusions(&word.parts, &mut excluded);

    merge_adjacent_spans(
        word_literal_scan_segments_excluding_expansions(word, source),
        source,
    )
    .into_iter()
    .find_map(|span| {
        let normalized = normalize_shell_quoting_segment_span(word, span, source);
        text_contains_shell_quoting_literals(normalized.slice(source))
            .then(|| shell_quoting_literal_run_span(word, normalized, &excluded, source))
    })
}

pub fn word_shell_quoting_literal_run_span_in_source(word: &Word, source: &str) -> Option<Span> {
    let text = word.span.slice(source);
    let mut cursor = if word.is_fully_double_quoted() && text.starts_with('"') {
        1
    } else {
        0
    };
    let limit = if word.is_fully_double_quoted() && text.ends_with('"') {
        text.len().saturating_sub(1)
    } else {
        text.len()
    };
    let mut saw_expansion = false;
    let mut in_single = false;
    let mut in_double = word.is_fully_double_quoted() && text.starts_with('"');
    let mut index = cursor;

    while index < limit {
        let tail = &text[index..limit];
        let Some(ch) = tail.chars().next() else {
            break;
        };
        if ch == '\'' && !in_double && !text_position_is_escaped(text, index) {
            in_single = !in_single;
            index += ch.len_utf8();
            continue;
        }
        if ch == '"' && !in_single && !text_position_is_escaped(text, index) {
            in_double = !in_double;
            index += ch.len_utf8();
            continue;
        }
        if !in_single && matches!(ch, '$' | '`') && !text_position_is_escaped(text, index) {
            saw_expansion = true;
            if let Some(span) = word_shell_quoting_segment_span_in_source(word, text, cursor, index)
            {
                return Some(span);
            }
            index += shell_quoting_expansion_len(tail);
            cursor = index;
            continue;
        }
        index += ch.len_utf8();
    }

    if let Some(span) = word_shell_quoting_segment_span_in_source(word, text, cursor, limit) {
        return Some(span);
    }
    if !saw_expansion && text_contains_shell_quoting_literals(&text[..limit]) {
        return Some(word.span);
    }

    None
}

pub fn word_double_quoted_scalar_only_expansion_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_double_quoted_scalar_only_expansion_spans(&word.parts, false, &mut spans)
        .then_some(spans)
        .filter(|spans| !spans.is_empty())
        .unwrap_or_default()
}

pub fn word_literal_part_spans_excluding_parameter_operator_tails(
    word: &Word,
    source: &str,
) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_word_literal_part_spans_excluding_parameter_operator_tails(word, source, &mut spans);
    spans
}

pub fn collect_word_literal_part_spans_excluding_parameter_operator_tails(
    word: &Word,
    source: &str,
    spans: &mut Vec<Span>,
) {
    spans.extend(
        word.parts
            .iter()
            .enumerate()
            .filter_map(|(index, part)| match &part.kind {
                WordPart::Literal(_)
                    if !literal_part_is_parameter_operator_tail(&word.parts, index, source) =>
                {
                    Some(part.span)
                }
                _ => None,
            }),
    );
}

pub fn word_has_single_literal_part(word: &Word) -> bool {
    matches!(
        word.parts.as_slice(),
        [part] if matches!(part.kind, WordPart::Literal(_))
    )
}

pub fn word_literal_scan_segments_excluding_expansions(word: &Word, source: &str) -> Vec<Span> {
    let mut excluded = Vec::new();
    collect_literal_scan_exclusions(&word.parts, &mut excluded);
    let mut spans = Vec::new();
    collect_scan_span_excluding(word.span, &excluded, source, &mut spans);
    spans
}

pub fn collect_word_literal_scan_segments_excluding_expansions(
    word: &Word,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let mut excluded = Vec::new();
    collect_literal_scan_exclusions(&word.parts, &mut excluded);
    collect_scan_span_excluding(word.span, &excluded, source, spans);
}

pub fn word_unquoted_glob_pattern_spans(word: &Word, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_unquoted_glob_pattern_spans(&word.parts, source, false, &mut spans);
    spans
}

pub fn word_unquoted_glob_pattern_spans_outside_brace_expansion(
    word: &Word,
    source: &str,
) -> Vec<Span> {
    let active_brace_spans = word
        .brace_syntax()
        .iter()
        .copied()
        .filter(|brace| brace.expands())
        .map(|brace| brace.span)
        .collect::<Vec<_>>();

    if active_brace_spans.is_empty() {
        return word_unquoted_glob_pattern_spans(word, source);
    }

    word_unquoted_glob_pattern_spans(word, source)
        .into_iter()
        .filter(|glob_span| {
            !active_brace_spans.iter().any(|brace_span| {
                brace_span.start.offset <= glob_span.start.offset
                    && glob_span.end.offset <= brace_span.end.offset
            })
        })
        .collect()
}

pub fn word_suspicious_bracket_glob_spans(word: &Word, source: &str) -> Vec<Span> {
    word_unquoted_glob_pattern_spans(word, source)
        .into_iter()
        .filter(|span| suspicious_bracket_glob_text(span.slice(source)))
        .collect()
}

pub fn word_has_unquoted_brace_expansion(word: &Word, source: &str) -> bool {
    parts_have_unquoted_brace_expansion(&word.parts, source, false)
}

pub fn word_unquoted_escaped_pipe_or_brace_spans_in_source(word: &Word, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_unquoted_escaped_pipe_or_brace_spans(&word.parts, source, false, &mut spans);
    spans
}

pub fn word_unbraced_variable_before_bracket_spans(word: &Word, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_unbraced_variable_before_bracket_spans(&word.parts, source, &mut spans);
    spans
}

pub fn word_standalone_literal_backslash_span(word: &Word, source: &str) -> Option<Span> {
    let [part] = word.parts.as_slice() else {
        return None;
    };
    if !matches!(part.kind, WordPart::Literal(_)) {
        return None;
    }

    let text = word.span.slice(source);
    let bytes = text.as_bytes();
    if bytes.len() != 2 || bytes[0] != b'\\' {
        return None;
    }

    let target = bytes[1];
    if !target.is_ascii_lowercase() || matches!(target, b'n' | b'r' | b't') {
        return None;
    }

    Some(Span::from_positions(word.span.start, word.span.start))
}

pub fn word_unquoted_star_parameter_spans(word: &Word, unquoted_array_spans: &[Span]) -> Vec<Span> {
    word.parts_with_spans()
        .filter_map(|(part, span)| {
            (unquoted_array_spans.contains(&span) && part_uses_star_splat(part)).then_some(span)
        })
        .collect()
}

pub fn word_unquoted_star_splat_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_unquoted_star_splat_spans(&word.parts, false, &mut spans);
    spans
}

pub fn word_quoted_star_splat_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_quoted_star_splat_spans(&word.parts, false, &mut spans);
    spans
}

pub fn word_unquoted_assign_default_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_unquoted_assign_default_spans(&word.parts, false, &mut spans);
    spans
}

pub fn word_use_replacement_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_use_replacement_spans(&word.parts, &mut spans);
    spans
}

pub fn word_unquoted_word_after_single_quoted_segment_spans(
    word: &Word,
    source: &str,
) -> Vec<Span> {
    let mut spans = Vec::new();

    for (index, part) in word.parts.iter().enumerate() {
        if !is_non_dollar_single_quoted(part) {
            continue;
        }
        if single_quoted_fragment_inner_text(part, source).is_some_and(|text| text.ends_with('\\'))
        {
            continue;
        }

        for next_part in word.parts.iter().skip(index + 1) {
            if next_part.kind.is_quoted() {
                break;
            }

            let WordPart::Literal(text) = &next_part.kind else {
                continue;
            };
            if literal_contains_unquoted_word_chars(text.as_str(source, next_part.span)) {
                spans.push(next_part.span);
            }
        }
    }

    spans
}

pub fn word_unquoted_scalar_between_double_quoted_segments_spans(
    word: &Word,
    candidate_spans: &[Span],
) -> Vec<Span> {
    if word.parts.len() < 3 {
        return Vec::new();
    }

    word.parts
        .windows(3)
        .filter_map(|window| {
            let [left, middle, right] = window else {
                return None;
            };

            (matches!(left.kind, WordPart::DoubleQuoted { .. })
                && candidate_spans.contains(&middle.span)
                && matches!(right.kind, WordPart::DoubleQuoted { .. }))
            .then_some(middle.span)
        })
        .collect()
}

pub fn word_nested_dynamic_double_quote_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_nested_dynamic_double_quote_spans(&word.parts, false, &mut spans);
    spans
}

pub fn word_positional_at_splat_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_positional_at_splat_spans(&word.parts, &mut spans);
    spans
}

pub fn word_is_pure_positional_at_splat(word: &Word) -> bool {
    parts_are_pure_positional_at_splat(&word.parts)
}

pub fn word_folded_positional_at_splat_span(word: &Word) -> Option<Span> {
    let spans = word_positional_at_splat_spans(word);
    if spans.is_empty() {
        return None;
    }
    if spans.len() == 1 && word_has_single_positional_at_splat_part(word) {
        return None;
    }

    spans.into_iter().next()
}

pub fn word_has_folded_positional_at_splat(word: &Word) -> bool {
    word_folded_positional_at_splat_span(word).is_some()
}

pub fn word_positional_at_splat_span_in_source(word: &Word, source: &str) -> Option<Span> {
    word_positional_at_splat_spans(word)
        .into_iter()
        .find(|span| !span_is_escaped(*span, source))
}

pub fn word_folded_positional_at_splat_span_in_source(word: &Word, source: &str) -> Option<Span> {
    let spans = word_positional_at_splat_spans(word)
        .into_iter()
        .filter(|span| !span_is_escaped(*span, source))
        .collect::<Vec<_>>();
    let first = spans.first().copied()?;

    if spans.len() == 1
        && (word_has_single_positional_at_splat_part(word)
            || positional_at_splat_is_standalone_expansion(word, source))
    {
        return None;
    }

    Some(first)
}

pub fn word_folded_all_elements_array_span_in_source(word: &Word, source: &str) -> Option<Span> {
    let spans = folded_all_elements_array_candidate_spans(word, source)
        .into_iter()
        .filter(|span| !span_is_escaped(*span, source))
        .collect::<Vec<_>>();
    let first = spans.first().copied()?;

    if spans.len() == 1
        && (word_has_single_folded_all_elements_array_part(word)
            || all_elements_array_expansion_is_standalone(word, source))
    {
        return None;
    }

    Some(first)
}

fn folded_all_elements_array_candidate_spans(word: &Word, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_folded_all_elements_array_candidate_spans(&word.parts, source, &mut spans);
    spans
}

fn collect_folded_all_elements_array_candidate_spans(
    parts: &[WordPartNode],
    source: &str,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_folded_all_elements_array_candidate_spans(parts, source, spans)
            }
            WordPart::Parameter(parameter)
                if parameter_uses_replacement_all_elements_array_expansion(parameter) =>
            {
                spans.push(part.span);
            }
            _ if part_uses_direct_all_elements_array_expansion(&part.kind) => {
                if let Some(span) =
                    normalize_direct_all_elements_array_expansion_span(part.span, source)
                {
                    spans.push(span);
                }
            }
            WordPart::Parameter(parameter)
                if parameter_might_use_all_elements_array_expansion(
                    parameter, part.span, source,
                ) =>
            {
                if let Some(span) =
                    normalize_nested_direct_all_elements_array_expansion_span(part.span, source)
                {
                    spans.push(span);
                }
            }
            _ => {}
        }
    }
}

pub fn word_zsh_flag_modifier_spans(word: &Word) -> Vec<Span> {
    word.parts
        .iter()
        .filter_map(|part| {
            let WordPart::Parameter(parameter) = &part.kind else {
                return None;
            };
            let ParameterExpansionSyntax::Zsh(syntax) = &parameter.syntax else {
                return None;
            };
            if syntax.modifiers.is_empty() {
                return None;
            }
            if syntax
                .modifiers
                .first()
                .is_some_and(|modifier| modifier.name == '=')
            {
                return None;
            }

            match syntax.target {
                ZshExpansionTarget::Reference(_) | ZshExpansionTarget::Word(_) => {}
                ZshExpansionTarget::Nested(_) | ZshExpansionTarget::Empty => return None,
            }

            syntax.modifiers.first().map(|modifier| modifier.span)
        })
        .collect()
}

pub fn word_zsh_nested_expansion_spans(word: &Word) -> Vec<Span> {
    word.parts
        .iter()
        .filter_map(|part| {
            let WordPart::Parameter(parameter) = &part.kind else {
                return None;
            };
            let ParameterExpansionSyntax::Zsh(syntax) = &parameter.syntax else {
                return None;
            };

            matches!(syntax.target, ZshExpansionTarget::Nested(_))
                .then_some(syntax.operation.is_none())
                .filter(|is_none| *is_none)
                .map(|_| parameter.span)
        })
        .collect()
}

pub fn word_nested_zsh_substitution_spans(word: &Word) -> Vec<Span> {
    word.parts
        .iter()
        .filter_map(|part| {
            let WordPart::Parameter(parameter) = &part.kind else {
                return None;
            };
            let ParameterExpansionSyntax::Zsh(syntax) = &parameter.syntax else {
                return None;
            };

            matches!(syntax.target, ZshExpansionTarget::Nested(_))
                .then_some(syntax.operation.as_ref())
                .flatten()
                .map(|_| parameter.span)
        })
        .collect()
}

pub fn conditional_extglob_span(expression: &ConditionalExpr, source: &str) -> Option<Span> {
    match expression {
        ConditionalExpr::Binary(expr) => conditional_extglob_span(&expr.left, source)
            .or_else(|| conditional_extglob_span(&expr.right, source)),
        ConditionalExpr::Unary(expr) => conditional_extglob_span(&expr.expr, source),
        ConditionalExpr::Parenthesized(expr) => conditional_extglob_span(&expr.expr, source),
        ConditionalExpr::Pattern(pattern) => pattern_extglob_span(pattern, source),
        ConditionalExpr::Word(_) | ConditionalExpr::Regex(_) | ConditionalExpr::VarRef(_) => None,
    }
}

pub fn conditional_array_subscript_span(
    expression: &ConditionalExpr,
    source: &str,
) -> Option<Span> {
    match expression {
        ConditionalExpr::Binary(expr) => conditional_array_subscript_span(&expr.left, source)
            .or_else(|| conditional_array_subscript_span(&expr.right, source)),
        ConditionalExpr::Unary(expr) => conditional_array_subscript_span(&expr.expr, source),
        ConditionalExpr::Parenthesized(expr) => {
            conditional_array_subscript_span(&expr.expr, source)
        }
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
            word_array_subscript_span(word, source)
        }
        ConditionalExpr::Pattern(pattern) => pattern_array_subscript_span(pattern, source),
        ConditionalExpr::VarRef(reference) => var_ref_subscript_span(reference),
    }
}

pub fn word_array_subscript_span(word: &Word, source: &str) -> Option<Span> {
    word_array_subscript_span_from_parts(&word.parts, source).or_else(|| {
        (!word.has_quoted_parts() && text_has_variable_subscript(word.span.slice(source)))
            .then_some(word.span)
    })
}

pub fn word_extglob_span(word: &Word, source: &str) -> Option<Span> {
    word_extglob_span_from_literal_parts(&word.parts, source).or_else(|| {
        if word_has_only_literal_parts(&word.parts) {
            return find_extglob_bounds(word.span.slice(source).as_bytes()).map(|_| word.span);
        }

        let (surface, source_offsets) = word_surface_bytes(word, source)?;
        let (start, end) = find_extglob_bounds(&surface)?;
        word_surface_span_from_bounds(word, source, &source_offsets, start, end)
    })
}

pub fn word_starts_with_extglob(word: &Word, source: &str) -> bool {
    if word_has_only_literal_parts(&word.parts) {
        return matches!(
            find_extglob_bounds(word.span.slice(source).as_bytes()),
            Some((0, _))
        );
    }

    let Some((surface, _)) = word_surface_bytes(word, source) else {
        return false;
    };

    matches!(find_extglob_bounds(&surface), Some((0, _)))
}

pub fn word_exactly_one_extglob_span(word: &Word, source: &str) -> Option<Span> {
    word_exactly_one_extglob_span_from_literal_parts(&word.parts, source).or_else(|| {
        if word_has_only_literal_parts(&word.parts) {
            return find_exactly_one_extglob_bounds(word.span.slice(source).as_bytes())
                .map(|_| word.span);
        }

        let (surface, source_offsets) = word_surface_bytes(word, source)?;
        let (start, end) = find_exactly_one_extglob_bounds(&surface)?;
        word_surface_span_from_bounds(word, source, &source_offsets, start, end)
    })
}

pub fn conditional_exactly_one_extglob_span(
    expression: &ConditionalExpr,
    source: &str,
) -> Option<Span> {
    match expression {
        ConditionalExpr::Binary(expr) => conditional_exactly_one_extglob_span(&expr.left, source)
            .or_else(|| conditional_exactly_one_extglob_span(&expr.right, source)),
        ConditionalExpr::Unary(expr) => conditional_exactly_one_extglob_span(&expr.expr, source),
        ConditionalExpr::Parenthesized(expr) => {
            conditional_exactly_one_extglob_span(&expr.expr, source)
        }
        ConditionalExpr::Pattern(pattern) => pattern_exactly_one_extglob_span(pattern, source),
        ConditionalExpr::Word(_) | ConditionalExpr::Regex(_) | ConditionalExpr::VarRef(_) => None,
    }
}

pub fn conditional_suspicious_bracket_glob_spans(
    expression: &ConditionalExpr,
    source: &str,
) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_conditional_suspicious_bracket_glob_spans(expression, source, &mut spans);
    spans
}

pub fn case_item_suspicious_bracket_glob_spans(item: &CaseItem, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    for pattern in &item.patterns {
        collect_pattern_suspicious_bracket_glob_spans(pattern, source, &mut spans);
    }
    spans
}

pub fn text_looks_like_caret_negated_bracket(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index + 1 < bytes.len() {
        if bytes[index] != b'['
            || byte_is_backslash_escaped(bytes, index)
            || bytes[index + 1] != b'^'
            || byte_is_backslash_escaped(bytes, index + 1)
        {
            index += 1;
            continue;
        }

        for close in index + 2..bytes.len() {
            if bytes[close] == b']' && !byte_is_backslash_escaped(bytes, close) {
                return true;
            }
        }

        index += 1;
    }

    false
}

pub fn word_caret_negated_bracket_spans(word: &Word, source: &str) -> Vec<Span> {
    if word_has_only_literal_parts(&word.parts) {
        let spans = word_caret_negated_bracket_spans_from_literal_parts(&word.parts, source);
        if !spans.is_empty() {
            return spans;
        }

        let text = word.span.slice(source);
        return find_caret_negated_bracket_bounds(text.as_bytes())
            .into_iter()
            .map(|(start, end)| {
                Span::from_positions(
                    word.span.start.advanced_by(&text[..start]),
                    word.span.start.advanced_by(&text[..end + 1]),
                )
            })
            .collect();
    }

    let Some((surface, source_offsets)) = word_surface_bytes(word, source) else {
        return Vec::new();
    };

    find_caret_negated_bracket_bounds(&surface)
        .into_iter()
        .filter_map(|(start, end)| {
            word_surface_span_from_bounds(word, source, &source_offsets, start, end)
        })
        .collect()
}

fn collect_command_substitution_spans(parts: &[WordPartNode], spans: &mut Vec<Span>) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                collect_command_substitution_spans(parts, spans)
            }
            WordPart::CommandSubstitution { .. } => spans.push(part.span),
            _ => {}
        }
    }
}

fn collect_conditional_suspicious_bracket_glob_spans(
    expression: &ConditionalExpr,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match expression {
        ConditionalExpr::Binary(expr) => {
            collect_conditional_suspicious_bracket_glob_spans(&expr.left, source, spans);
            collect_conditional_suspicious_bracket_glob_spans(&expr.right, source, spans);
        }
        ConditionalExpr::Unary(expr) => {
            collect_conditional_suspicious_bracket_glob_spans(&expr.expr, source, spans);
        }
        ConditionalExpr::Parenthesized(expr) => {
            collect_conditional_suspicious_bracket_glob_spans(&expr.expr, source, spans);
        }
        ConditionalExpr::Pattern(pattern) => {
            collect_pattern_suspicious_bracket_glob_spans(pattern, source, spans);
        }
        ConditionalExpr::Word(_) | ConditionalExpr::Regex(_) | ConditionalExpr::VarRef(_) => {}
    }
}

fn collect_pattern_suspicious_bracket_glob_spans(
    pattern: &Pattern,
    source: &str,
    spans: &mut Vec<Span>,
) {
    for (part, span) in pattern.parts_with_spans() {
        match part {
            PatternPart::Group { patterns, .. } => {
                for pattern in patterns {
                    collect_pattern_suspicious_bracket_glob_spans(pattern, source, spans);
                }
            }
            PatternPart::Word(word) => {
                spans.extend(word_suspicious_bracket_glob_spans(word, source))
            }
            PatternPart::CharClass(_) if suspicious_bracket_glob_text(span.slice(source)) => {
                spans.push(span);
            }
            PatternPart::CharClass(_)
            | PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar => {}
        }
    }
}

fn collect_arithmetic_expansion_spans(parts: &[WordPartNode], spans: &mut Vec<Span>) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                collect_arithmetic_expansion_spans(parts, spans)
            }
            WordPart::ArithmeticExpansion { .. } => spans.push(part.span),
            _ => {}
        }
    }
}

fn collect_parenthesized_arithmetic_expansion_spans(parts: &[WordPartNode], spans: &mut Vec<Span>) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                collect_parenthesized_arithmetic_expansion_spans(parts, spans)
            }
            WordPart::ArithmeticExpansion {
                expression_ast: Some(expression),
                ..
            } => {
                if matches!(expression.kind, ArithmeticExpr::Parenthesized { .. }) {
                    spans.push(expression.span);
                }
            }
            WordPart::ArithmeticExpansion {
                expression_ast: None,
                ..
            } => {}
            _ => {}
        }
    }
}

fn collect_unquoted_command_substitution_spans(
    parts: &[WordPartNode],
    quoted: bool,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_unquoted_command_substitution_spans(parts, true, spans)
            }
            WordPart::CommandSubstitution { .. } if !quoted => spans.push(part.span),
            _ => {}
        }
    }
}

fn collect_unquoted_dollar_paren_command_substitution_spans(
    parts: &[WordPartNode],
    quoted: bool,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_unquoted_dollar_paren_command_substitution_spans(parts, true, spans)
            }
            WordPart::CommandSubstitution {
                syntax: CommandSubstitutionSyntax::DollarParen,
                ..
            } if !quoted => spans.push(part.span),
            _ => {}
        }
    }
}

fn collect_array_expansion_spans(
    parts: &[WordPartNode],
    quoted: bool,
    only_unquoted: bool,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_array_expansion_spans(parts, true, only_unquoted, spans)
            }
            WordPart::Variable(name)
                if matches!(name.as_str(), "@" | "*") && (!quoted || !only_unquoted) =>
            {
                spans.push(part.span);
            }
            WordPart::ArrayAccess(reference)
                if reference.has_array_selector() && (!quoted || !only_unquoted) =>
            {
                spans.push(part.span);
            }
            WordPart::Parameter(parameter)
                if parameter_is_array_like(parameter) && (!quoted || !only_unquoted) =>
            {
                spans.push(part.span);
            }
            WordPart::ParameterExpansion {
                reference,
                operator,
                ..
            } if !matches!(operator, ParameterOp::UseReplacement)
                && reference.has_array_selector()
                && (!quoted || !only_unquoted) =>
            {
                spans.push(part.span);
            }
            WordPart::IndirectExpansion {
                reference,
                operator,
                ..
            } if !matches!(operator, Some(ParameterOp::UseReplacement))
                && reference.has_array_selector()
                && (!quoted || !only_unquoted) =>
            {
                spans.push(part.span);
            }
            WordPart::Transformation { reference, .. }
                if reference.has_array_selector() && (!quoted || !only_unquoted) =>
            {
                spans.push(part.span);
            }
            WordPart::ArraySlice { .. } | WordPart::ArrayIndices(_)
                if !quoted || !only_unquoted =>
            {
                spans.push(part.span);
            }
            _ => {}
        }
    }
}

fn collect_all_elements_array_expansion_spans(
    parts: &[WordPartNode],
    source: &str,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_all_elements_array_expansion_spans(parts, source, spans)
            }
            WordPart::Variable(name) if name.as_str() == "@" => {
                if let Some(span) = normalize_all_elements_array_expansion_span(part.span, source) {
                    spans.push(span);
                }
            }
            WordPart::ArrayAccess(reference)
                if matches!(
                    reference
                        .subscript
                        .as_ref()
                        .and_then(|subscript| subscript.selector()),
                    Some(SubscriptSelector::At)
                ) =>
            {
                if let Some(span) = normalize_all_elements_array_expansion_span(part.span, source) {
                    spans.push(span);
                }
            }
            WordPart::ArrayIndices(reference)
                if matches!(
                    reference
                        .subscript
                        .as_ref()
                        .and_then(|subscript| subscript.selector()),
                    Some(SubscriptSelector::At)
                ) =>
            {
                if let Some(span) = normalize_all_elements_array_expansion_span(part.span, source) {
                    spans.push(span);
                }
            }
            WordPart::PrefixMatch {
                kind: PrefixMatchKind::At,
                ..
            } => {
                if let Some(span) = normalize_all_elements_array_expansion_span(part.span, source) {
                    spans.push(span);
                }
            }
            WordPart::Parameter(parameter)
                if parameter_might_use_all_elements_array_expansion(
                    parameter, part.span, source,
                ) =>
            {
                if let Some(span) = normalize_all_elements_array_expansion_span(part.span, source) {
                    spans.push(span);
                }
            }
            WordPart::Variable(name) if name.as_str() == "*" => {}
            _ => {}
        }
    }
}

fn parts_have_all_elements_array_expansion_syntax(parts: &[WordPartNode]) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::SingleQuoted { .. } => false,
        WordPart::DoubleQuoted { parts, .. } => {
            parts_have_all_elements_array_expansion_syntax(parts)
        }
        WordPart::Variable(name) => name.as_str() == "@",
        WordPart::ArrayAccess(reference) | WordPart::ArrayIndices(reference) => {
            var_ref_uses_all_elements_at_splat(reference)
        }
        WordPart::ArraySlice { reference, .. } => var_ref_uses_all_elements_at_splat(reference),
        WordPart::PrefixMatch {
            kind: PrefixMatchKind::At,
            ..
        } => true,
        WordPart::PrefixMatch {
            kind: PrefixMatchKind::Star,
            ..
        } => false,
        WordPart::Parameter(parameter) => {
            parameter_uses_unquoted_all_elements_array_expansion(parameter)
        }
        WordPart::Literal(_)
        | WordPart::CommandSubstitution { .. }
        | WordPart::ArithmeticExpansion { .. }
        | WordPart::Length(_)
        | WordPart::ParameterExpansion { .. }
        | WordPart::IndirectExpansion { .. }
        | WordPart::ProcessSubstitution { .. }
        | WordPart::Transformation { .. }
        | WordPart::Substring { .. }
        | WordPart::ArrayLength(_)
        | WordPart::ZshQualifiedGlob(_) => false,
    })
}

fn collect_unquoted_all_elements_array_expansion_spans(
    parts: &[WordPartNode],
    quoted: bool,
    _source: &str,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_unquoted_all_elements_array_expansion_spans(parts, true, _source, spans)
            }
            _ if !quoted && part_uses_unquoted_all_elements_array_expansion(&part.kind) => {
                spans.push(part.span)
            }
            _ => {}
        }
    }
}

fn collect_all_elements_array_slice_spans(
    parts: &[WordPartNode],
    quoted: bool,
    only_quoted: bool,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_all_elements_array_slice_spans(parts, true, only_quoted, spans)
            }
            _ if (!only_quoted || quoted) && part_uses_all_elements_array_slice(&part.kind) => {
                spans.push(part.span)
            }
            _ => {}
        }
    }
}

fn collect_direct_all_elements_array_expansion_spans(
    parts: &[WordPartNode],
    source: &str,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_direct_all_elements_array_expansion_spans(parts, source, spans)
            }
            _ if part_uses_direct_all_elements_array_expansion(&part.kind) => {
                if let Some(span) =
                    normalize_direct_all_elements_array_expansion_span(part.span, source)
                {
                    spans.push(span);
                }
            }
            WordPart::Parameter(parameter)
                if parameter_might_use_all_elements_array_expansion(
                    parameter, part.span, source,
                ) =>
            {
                if let Some(span) =
                    normalize_nested_direct_all_elements_array_expansion_span(part.span, source)
                {
                    spans.push(span);
                }
            }
            _ => {}
        }
    }
}

fn collect_quoted_unindexed_bash_source_spans(
    parts: &[WordPartNode],
    quoted: bool,
    source: &str,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_quoted_unindexed_bash_source_spans(parts, true, source, spans)
            }
            WordPart::Variable(name)
                if quoted
                    && name.as_str() == "BASH_SOURCE"
                    && !span_is_escaped(part.span, source) =>
            {
                spans.push(part.span);
            }
            WordPart::Parameter(parameter)
                if quoted
                    && parameter_is_unindexed_bash_source(parameter)
                    && !span_is_escaped(part.span, source) =>
            {
                spans.push(part.span);
            }
            _ => {}
        }
    }
}

fn normalize_all_elements_array_expansion_span(span: Span, source: &str) -> Option<Span> {
    let text = span.slice(source);
    if !span_is_escaped(span, source)
        && (text == "$@" || candidate_is_all_elements_array_expansion(text))
    {
        return Some(span);
    }

    let base_offset = span.start.offset;
    let mut search_from = 0usize;

    while let Some(found) = text[search_from..].find('$') {
        let relative_start = search_from + found;
        let absolute_start = base_offset + relative_start;
        if offset_is_backslash_escaped(absolute_start, source) {
            search_from = relative_start + 1;
            continue;
        }

        let start = position_at_offset(source, absolute_start)?;
        let remainder = &source[absolute_start..];

        if remainder.starts_with("$@") {
            let end = position_at_offset(source, absolute_start + "$@".len())?;
            return Some(Span::from_positions(start, end));
        }

        if remainder.starts_with("${")
            && let Some(relative_end) = remainder.find('}')
        {
            let candidate = &remainder[..=relative_end];
            if candidate_is_all_elements_array_expansion(candidate) {
                let end = position_at_offset(source, absolute_start + candidate.len())?;
                return Some(Span::from_positions(start, end));
            }
        }

        search_from = relative_start + 1;
    }

    widen_all_elements_array_expansion_span(span, source)
}

fn normalize_direct_all_elements_array_expansion_span(span: Span, source: &str) -> Option<Span> {
    let text = span.slice(source);
    if !span_is_escaped(span, source)
        && (text == "$@" || candidate_is_direct_all_elements_array_expansion(text))
    {
        return Some(span);
    }

    let base_offset = span.start.offset;
    let mut search_from = 0usize;

    while let Some(found) = text[search_from..].find('$') {
        let relative_start = search_from + found;
        let absolute_start = base_offset + relative_start;
        if offset_is_backslash_escaped(absolute_start, source) {
            search_from = relative_start + 1;
            continue;
        }

        let start = position_at_offset(source, absolute_start)?;
        let remainder = &source[absolute_start..];

        if remainder.starts_with("$@") {
            let end = position_at_offset(source, absolute_start + "$@".len())?;
            return Some(Span::from_positions(start, end));
        }

        if remainder.starts_with("${")
            && let Some(relative_end) = remainder.find('}')
        {
            let candidate = &remainder[..=relative_end];
            if candidate_is_direct_all_elements_array_expansion(candidate) {
                let end = position_at_offset(source, absolute_start + candidate.len())?;
                return Some(Span::from_positions(start, end));
            }
        }

        search_from = relative_start + 1;
    }

    widen_direct_all_elements_array_expansion_span(span, source)
}

fn normalize_nested_direct_all_elements_array_expansion_span(
    span: Span,
    source: &str,
) -> Option<Span> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum QuoteState {
        None,
        Single,
        Double,
    }

    let text = span.slice(source);
    if !text.contains('$') {
        return None;
    }

    let base_offset = span.start.offset;
    let bytes = text.as_bytes();
    let mut index = 0usize;
    let mut nested_braced_depth = 0usize;
    let mut quote_state = QuoteState::None;

    while index < bytes.len() {
        let absolute_start = base_offset + index;
        let byte = bytes[index];

        match quote_state {
            QuoteState::Single if nested_braced_depth > 0 => {
                if byte == b'\'' {
                    quote_state = QuoteState::None;
                }
                index += 1;
                continue;
            }
            QuoteState::Double if nested_braced_depth > 0 => {
                if byte == b'\\' {
                    index += usize::from(index + 1 < bytes.len()) + 1;
                    continue;
                }
                if byte == b'"' {
                    quote_state = QuoteState::None;
                }
                index += 1;
                continue;
            }
            QuoteState::None if nested_braced_depth > 0 && byte == b'\'' => {
                quote_state = QuoteState::Single;
                index += 1;
                continue;
            }
            QuoteState::None if nested_braced_depth > 0 && byte == b'"' => {
                quote_state = QuoteState::Double;
                index += 1;
                continue;
            }
            QuoteState::None => {}
            QuoteState::Single | QuoteState::Double => {}
        }

        if byte == b'\\' {
            if index + 2 < bytes.len() && bytes[index + 1] == b'$' && bytes[index + 2] == b'{' {
                nested_braced_depth += 1;
                index += 3;
                continue;
            }

            index += usize::from(index + 1 < bytes.len()) + 1;
            continue;
        }

        if byte == b'}' && nested_braced_depth > 0 {
            nested_braced_depth -= 1;
            index += 1;
            continue;
        }

        if byte != b'$' {
            if byte == b'{' && nested_braced_depth > 0 {
                nested_braced_depth += 1;
            }
            index += 1;
            continue;
        }

        if offset_is_backslash_escaped(absolute_start, source) {
            index += 1;
            continue;
        }

        let remainder = &source[absolute_start..];
        if nested_braced_depth == 0 && remainder.starts_with("$@") {
            let start = position_at_offset(source, absolute_start)?;
            let end = position_at_offset(source, absolute_start + "$@".len())?;
            return Some(Span::from_positions(start, end));
        }

        if remainder.starts_with("${") {
            if nested_braced_depth == 0
                && let Some(relative_end) = remainder.find('}')
            {
                let candidate = &remainder[..=relative_end];
                if candidate_is_direct_all_elements_array_expansion(candidate) {
                    let start = position_at_offset(source, absolute_start)?;
                    let end = position_at_offset(source, absolute_start + candidate.len())?;
                    return Some(Span::from_positions(start, end));
                }
            }

            nested_braced_depth += 1;
            index += 2;
            continue;
        }

        index += 1;
    }

    None
}

fn normalize_command_substitution_span(span: Span, source: &str) -> Span {
    let text = span.slice(source);
    if text.starts_with("$(")
        && !text.ends_with(')')
        && let Some(normalized) = widen_dollar_paren_command_substitution_span(span, source)
    {
        return normalized;
    }

    if text.starts_with('`')
        && !text.ends_with('`')
        && let Some(normalized) = widen_backtick_command_substitution_span(span, source)
    {
        return normalized;
    }

    span
}

fn normalize_command_substitution_spans(spans: &mut [Span], source: &str) {
    for span in spans {
        *span = normalize_command_substitution_span(*span, source);
    }
}

fn collapse_backtick_continuation_span(
    span: Span,
    source: &str,
    backtick_spans: &[Span],
) -> Option<Span> {
    let containing_span = containing_backtick_substitution_span(span, backtick_spans)?;
    let chain_start = continued_line_chain_start(span.start, containing_span, source)?;
    Some(Span::from_positions(
        shellcheck_collapsed_position(chain_start, span.start, source),
        shellcheck_collapsed_position(chain_start, span.end, source),
    ))
}

fn shellcheck_deescaped_backtick_part_span(
    span: Span,
    source: &str,
    backtick_spans: &[Span],
) -> Option<Span> {
    let containing_span = containing_backtick_substitution_span(span, backtick_spans)?;
    let content_start = containing_span.start.offset.saturating_add('`'.len_utf8());
    let start_removed = backtick_removed_escape_count(source, content_start, span.start.offset)?;
    let end_removed = backtick_removed_escape_count(source, content_start, span.end.offset)?;
    if start_removed == 0 && end_removed == 0 {
        return None;
    }

    Some(Span::from_positions(
        position_at_offset(source, span.start.offset.checked_sub(start_removed)?)?,
        position_at_offset(source, span.end.offset.checked_sub(end_removed)?)?,
    ))
}

fn backtick_removed_escape_count(source: &str, start: usize, end: usize) -> Option<usize> {
    let mut removed = 0usize;
    let mut index = start;
    while index < end {
        let ch = source[index..].chars().next()?;
        let ch_len = ch.len_utf8();
        if ch != '\\' {
            index += ch_len;
            continue;
        }

        let next_offset = index + ch_len;
        if next_offset >= end {
            break;
        }
        let escaped = source[next_offset..].chars().next()?;
        if matches!(escaped, '$' | '`' | '\\') {
            removed += 1;
            index = next_offset + escaped.len_utf8();
        } else {
            index += ch_len;
        }
    }

    Some(removed)
}

fn containing_backtick_substitution_span(target: Span, backtick_spans: &[Span]) -> Option<Span> {
    backtick_spans
        .iter()
        .copied()
        .find(|span| span_contains(*span, target))
}

#[derive(Clone, Copy, Default)]
struct BacktickQuoteContext {
    in_single_quote: bool,
    in_double_quote: bool,
    in_comment: bool,
    previous_char: Option<char>,
}

fn backtick_shell_comment_can_start(previous_char: Option<char>) -> bool {
    previous_char.is_none_or(|ch| {
        ch.is_ascii_whitespace() || matches!(ch, ';' | '|' | '&' | '(' | ')' | '<' | '>')
    })
}

pub(crate) fn backtick_substitution_spans(source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut contexts = vec![BacktickQuoteContext::default()];
    let mut backtick_start_offsets = Vec::<usize>::new();
    let mut index = 0usize;

    while index < source.len() {
        let ch = source[index..]
            .chars()
            .next()
            .expect("index should remain on UTF-8 boundaries");
        let ch_len = ch.len_utf8();

        if contexts
            .last()
            .expect("scanner should always retain a root context")
            .in_comment
        {
            if ch == '\n' {
                let context = contexts
                    .last_mut()
                    .expect("scanner should always retain a root context");
                context.in_comment = false;
                context.previous_char = Some(ch);
            }
            index += ch_len;
            continue;
        }

        if contexts
            .last()
            .expect("scanner should always retain a root context")
            .in_single_quote
        {
            if ch == '\'' {
                contexts
                    .last_mut()
                    .expect("scanner should always retain a root context")
                    .in_single_quote = false;
            }
            contexts
                .last_mut()
                .expect("scanner should always retain a root context")
                .previous_char = Some(ch);
            index += ch_len;
            continue;
        }

        match ch {
            '\\' => {
                index += ch_len;
                if index < source.len() {
                    let escaped = source[index..]
                        .chars()
                        .next()
                        .expect("index should remain on UTF-8 boundaries");
                    index += escaped.len_utf8();
                    contexts
                        .last_mut()
                        .expect("scanner should always retain a root context")
                        .previous_char = Some(escaped);
                } else {
                    contexts
                        .last_mut()
                        .expect("scanner should always retain a root context")
                        .previous_char = Some('\\');
                }
            }
            '\'' if !contexts
                .last()
                .expect("scanner should always retain a root context")
                .in_double_quote =>
            {
                let context = contexts
                    .last_mut()
                    .expect("scanner should always retain a root context");
                context.in_single_quote = true;
                context.previous_char = Some(ch);
                index += ch_len;
            }
            '"' => {
                let context = contexts
                    .last_mut()
                    .expect("scanner should always retain a root context");
                context.in_double_quote = !context.in_double_quote;
                context.previous_char = Some(ch);
                index += ch_len;
            }
            '#' if !contexts
                .last()
                .expect("scanner should always retain a root context")
                .in_double_quote
                && backtick_shell_comment_can_start(
                    contexts
                        .last()
                        .expect("scanner should always retain a root context")
                        .previous_char,
                ) =>
            {
                contexts
                    .last_mut()
                    .expect("scanner should always retain a root context")
                    .in_comment = true;
                index += ch_len;
            }
            '`' => {
                if let Some(start_offset) = backtick_start_offsets.pop() {
                    let _ = contexts.pop();
                    let Some(start) = position_at_offset(source, start_offset) else {
                        index += ch_len;
                        continue;
                    };
                    let Some(end) = position_at_offset(source, index + ch_len) else {
                        index += ch_len;
                        continue;
                    };
                    spans.push(Span::from_positions(start, end));
                    contexts
                        .last_mut()
                        .expect("scanner should always retain a root context")
                        .previous_char = Some(ch);
                } else {
                    backtick_start_offsets.push(index);
                    contexts
                        .last_mut()
                        .expect("scanner should always retain a root context")
                        .previous_char = Some(ch);
                    contexts.push(BacktickQuoteContext::default());
                }
                index += ch_len;
            }
            _ => {
                contexts
                    .last_mut()
                    .expect("scanner should always retain a root context")
                    .previous_char = Some(ch);
                index += ch_len;
            }
        }
    }

    spans
}

pub(crate) fn backtick_escaped_parameters(
    source: &str,
    backtick_spans: &[Span],
) -> Vec<BacktickEscapedParameter> {
    let mut spans = Vec::new();

    for backtick_span in backtick_spans {
        let mut index = backtick_span.start.offset.saturating_add('`'.len_utf8());
        let end = backtick_span.end.offset.saturating_sub('`'.len_utf8());
        let mut in_single_quote = false;
        let mut in_double_quote = false;
        let mut removed_escapes = 0usize;

        while index < end {
            let ch = source[index..]
                .chars()
                .next()
                .expect("index should remain on UTF-8 boundaries");
            let ch_len = ch.len_utf8();

            match ch {
                '\'' if !in_double_quote => {
                    in_single_quote = !in_single_quote;
                    index += ch_len;
                }
                '"' if !in_single_quote => {
                    in_double_quote = !in_double_quote;
                    index += ch_len;
                }
                '\\' if !in_single_quote => {
                    let slash_offset = index;
                    index += ch_len;
                    if index >= end {
                        continue;
                    }

                    let escaped = source[index..]
                        .chars()
                        .next()
                        .expect("index should remain on UTF-8 boundaries");
                    if escaped == '$'
                        && !in_double_quote
                        && let Some(parameter) =
                            escaped_backtick_parameter_syntax(source, index, end)
                    {
                        let expansion_len = parameter.expansion_len();
                        let diagnostic_start_offset = slash_offset.saturating_sub(removed_escapes);
                        let Some(diagnostic_start) =
                            position_at_offset(source, diagnostic_start_offset)
                        else {
                            index += escaped.len_utf8();
                            continue;
                        };
                        let Some(diagnostic_end) =
                            position_at_offset(source, diagnostic_start_offset + expansion_len)
                        else {
                            index += escaped.len_utf8();
                            continue;
                        };
                        let Some(reference_start) = position_at_offset(source, index) else {
                            index += escaped.len_utf8();
                            continue;
                        };
                        let Some(reference_end) = position_at_offset(source, index + expansion_len)
                        else {
                            index += escaped.len_utf8();
                            continue;
                        };
                        spans.push(BacktickEscapedParameter {
                            name: parameter.name().cloned(),
                            diagnostic_span: Span::from_positions(diagnostic_start, diagnostic_end),
                            reference_span: Span::from_positions(reference_start, reference_end),
                            standalone_command_name:
                                escaped_backtick_parameter_is_standalone_command_name(
                                    source,
                                    *backtick_span,
                                    slash_offset,
                                    index + expansion_len,
                                ),
                        });
                        removed_escapes += 1;
                        index += expansion_len;
                    } else {
                        index += escaped.len_utf8();
                    }
                }
                _ => {
                    index += ch_len;
                }
            }
        }
    }

    spans.sort_by_key(|parameter| {
        (
            parameter.diagnostic_span.start.offset,
            parameter.diagnostic_span.end.offset,
            parameter.reference_span.start.offset,
            parameter.reference_span.end.offset,
        )
    });
    spans.dedup();
    spans
}

pub(crate) fn backtick_double_escaped_parameter_spans(
    source: &str,
    backtick_spans: &[Span],
) -> Vec<Span> {
    let mut spans = Vec::new();

    for backtick_span in backtick_spans {
        let mut index = backtick_span.start.offset.saturating_add('`'.len_utf8());
        let end = backtick_span.end.offset.saturating_sub('`'.len_utf8());
        let mut in_single_quote = false;
        let mut in_double_quote = false;

        while index < end {
            let ch = source[index..]
                .chars()
                .next()
                .expect("index should remain on UTF-8 boundaries");
            let ch_len = ch.len_utf8();

            match ch {
                '\'' if !in_double_quote => {
                    in_single_quote = !in_single_quote;
                    index += ch_len;
                }
                '"' if !in_single_quote => {
                    in_double_quote = !in_double_quote;
                    index += ch_len;
                }
                '\\' if !in_single_quote => {
                    let slash_start = index;
                    while index < end && source.as_bytes().get(index) == Some(&b'\\') {
                        index += '\\'.len_utf8();
                    }
                    let slash_count = index.saturating_sub(slash_start);
                    if in_double_quote
                        && slash_count == 2
                        && source.as_bytes().get(index) == Some(&b'$')
                        && let Some(parameter) =
                            escaped_backtick_parameter_syntax(source, index, end)
                    {
                        let expansion_len = parameter.expansion_len();
                        if let Some(start) = position_at_offset(source, index)
                            && let Some(end_position) =
                                position_at_offset(source, index + expansion_len)
                        {
                            spans.push(Span::from_positions(start, end_position));
                        }
                        index += expansion_len;
                    } else if slash_count % 2 == 1 && index < end {
                        let escaped = source[index..]
                            .chars()
                            .next()
                            .expect("index should remain on UTF-8 boundaries");
                        index += escaped.len_utf8();
                    }
                }
                _ => {
                    index += ch_len;
                }
            }
        }
    }

    spans.sort_by_key(|span| (span.start.offset, span.end.offset));
    spans.dedup();
    spans
}

fn escaped_backtick_parameter_is_standalone_command_name(
    source: &str,
    backtick_span: Span,
    slash_offset: usize,
    expansion_end: usize,
) -> bool {
    let segment_start = backtick_command_segment_start(
        source,
        backtick_span.start.offset.saturating_add('`'.len_utf8()),
        slash_offset,
    );
    let Some(prefix) = source.get(segment_start..slash_offset) else {
        return false;
    };
    if !command_prefix_is_empty_or_assignments(prefix) {
        return false;
    }

    let suffix_limit = backtick_span.end.offset.saturating_sub('`'.len_utf8());
    escaped_reference_ends_standalone_word(source, expansion_end, suffix_limit)
}

fn backtick_command_segment_start(source: &str, start: usize, end: usize) -> usize {
    let bytes = source.as_bytes();
    let mut cursor = start;
    let mut segment_start = start;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;

    while cursor < end {
        let byte = bytes[cursor];
        if escaped {
            escaped = false;
            cursor += 1;
            continue;
        }
        if byte == b'\\' && !in_single_quote {
            escaped = true;
            cursor += 1;
            continue;
        }

        match byte {
            b'\'' if !in_double_quote => in_single_quote = !in_single_quote,
            b'"' if !in_single_quote => in_double_quote = !in_double_quote,
            b'\n' | b';' if !in_single_quote && !in_double_quote => {
                segment_start = cursor + 1;
            }
            b'&' | b'|' if !in_single_quote && !in_double_quote => {
                let separator = byte;
                cursor += 1;
                while cursor < end && bytes[cursor] == separator {
                    cursor += 1;
                }
                segment_start = cursor;
                continue;
            }
            _ => {}
        }

        cursor += 1;
    }

    segment_start
}

fn command_prefix_is_empty_or_assignments(prefix: &str) -> bool {
    let mut index = 0;
    while index < prefix.len() {
        skip_shell_whitespace(prefix.as_bytes(), &mut index);
        if index >= prefix.len() {
            return true;
        }

        let Some(end) = shell_word_end(prefix, index) else {
            return false;
        };
        if simple_assignment_word(&prefix[index..end]) {
            index = end;
            continue;
        }
        if let Some(redirection_end) = redirection_prefix_end(prefix, index) {
            index = redirection_end;
            continue;
        }
        return false;
    }
    true
}

fn skip_shell_whitespace(bytes: &[u8], index: &mut usize) {
    while *index < bytes.len() && bytes[*index].is_ascii_whitespace() {
        *index += 1;
    }
}

fn shell_word_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut index = start;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while index < bytes.len() {
        let byte = bytes[index];
        if in_single_quote {
            if byte == b'\'' {
                in_single_quote = false;
            }
            index += 1;
            continue;
        }

        if byte == b'\\' {
            index = advance_escaped_shell_char(text, index);
            continue;
        }

        if !in_double_quote && byte.is_ascii_whitespace() {
            break;
        }

        match byte {
            b'\'' if !in_double_quote => {
                in_single_quote = true;
                index += 1;
            }
            b'"' => {
                in_double_quote = !in_double_quote;
                index += 1;
            }
            b'$' if bytes.get(index + 1) == Some(&b'(') => {
                index = skip_balanced_shell_construct(text, index + 2, b'(', b')')?;
            }
            b'$' if bytes.get(index + 1) == Some(&b'{') => {
                index = skip_balanced_shell_construct(text, index + 2, b'{', b'}')?;
            }
            b'<' | b'>' if !in_double_quote && bytes.get(index + 1) == Some(&b'(') => {
                index = skip_balanced_shell_construct(text, index + 2, b'(', b')')?;
            }
            b'`' => {
                index = skip_legacy_backtick_construct(text, index + 1)?;
            }
            _ => index = advance_shell_char(text, index),
        }
    }

    (!in_single_quote && !in_double_quote).then_some(index)
}

fn skip_balanced_shell_construct(
    text: &str,
    mut index: usize,
    open: u8,
    close: u8,
) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 1usize;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while index < bytes.len() {
        let byte = bytes[index];
        if in_single_quote {
            if byte == b'\'' {
                in_single_quote = false;
            }
            index += 1;
            continue;
        }

        if byte == b'\\' {
            index = advance_escaped_shell_char(text, index);
            continue;
        }

        match byte {
            b'\'' if !in_double_quote => {
                in_single_quote = true;
                index += 1;
            }
            b'"' => {
                in_double_quote = !in_double_quote;
                index += 1;
            }
            b'$' if bytes.get(index + 1) == Some(&b'(') => {
                index = skip_balanced_shell_construct(text, index + 2, b'(', b')')?;
            }
            b'$' if bytes.get(index + 1) == Some(&b'{') => {
                index = skip_balanced_shell_construct(text, index + 2, b'{', b'}')?;
            }
            b'<' | b'>' if !in_double_quote && bytes.get(index + 1) == Some(&b'(') => {
                index = skip_balanced_shell_construct(text, index + 2, b'(', b')')?;
            }
            b'`' => {
                index = skip_legacy_backtick_construct(text, index + 1)?;
            }
            _ if byte == open && !in_double_quote => {
                depth += 1;
                index += 1;
            }
            _ if byte == close && !in_double_quote => {
                depth -= 1;
                index += 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => index = advance_shell_char(text, index),
        }
    }

    None
}

fn skip_legacy_backtick_construct(text: &str, mut index: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while index < bytes.len() {
        let byte = bytes[index];
        if in_single_quote {
            if byte == b'\'' {
                in_single_quote = false;
            }
            index += 1;
            continue;
        }

        if byte == b'\\' {
            index = advance_escaped_shell_char(text, index);
            continue;
        }

        match byte {
            b'\'' if !in_double_quote => {
                in_single_quote = true;
                index += 1;
            }
            b'"' => {
                in_double_quote = !in_double_quote;
                index += 1;
            }
            b'`' if !in_double_quote => return Some(index + 1),
            _ => index = advance_shell_char(text, index),
        }
    }

    None
}

fn advance_escaped_shell_char(text: &str, index: usize) -> usize {
    let next = advance_shell_char(text, index);
    if next < text.len() {
        advance_shell_char(text, next)
    } else {
        next
    }
}

fn advance_shell_char(text: &str, index: usize) -> usize {
    text[index..]
        .chars()
        .next()
        .map_or(index + 1, |ch| index + ch.len_utf8())
}

fn simple_assignment_word(word: &str) -> bool {
    let Some(eq) = word.find('=') else {
        return false;
    };
    let name = word[..eq].strip_suffix('+').unwrap_or(&word[..eq]);
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn redirection_prefix_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut operator_start = start;
    while operator_start < bytes.len() && bytes[operator_start].is_ascii_digit() {
        operator_start += 1;
    }

    let operator_len = redirection_operator_len(text.get(operator_start..)?)?;
    let mut target_start = operator_start + operator_len;
    skip_shell_whitespace(bytes, &mut target_start);
    if target_start >= text.len() {
        return None;
    }

    shell_word_end(text, target_start)
}

fn redirection_operator_len(text: &str) -> Option<usize> {
    [
        "&>>", "<<<", "<>", ">>", "<<", "<&", ">&", ">|", "&>", "<", ">",
    ]
    .into_iter()
    .find(|operator| text.starts_with(operator))
    .map(str::len)
}

fn escaped_reference_ends_standalone_word(source: &str, start: usize, limit: usize) -> bool {
    let Some(rest) = source.get(start..limit) else {
        return false;
    };
    rest.chars().next().is_none_or(|ch| {
        ch.is_whitespace() || matches!(ch, ';' | '&' | '|' | '<' | '>' | '(' | ')')
    })
}

enum EscapedBacktickParameterSyntax {
    Simple {
        name: shuck_ast::Name,
        expansion_len: usize,
    },
    ComplexUnsafe {
        expansion_len: usize,
    },
}

impl EscapedBacktickParameterSyntax {
    fn name(&self) -> Option<&shuck_ast::Name> {
        match self {
            Self::Simple { name, .. } => Some(name),
            Self::ComplexUnsafe { .. } => None,
        }
    }

    fn expansion_len(&self) -> usize {
        match self {
            Self::Simple { expansion_len, .. } | Self::ComplexUnsafe { expansion_len } => {
                *expansion_len
            }
        }
    }
}

fn escaped_backtick_parameter_syntax(
    source: &str,
    dollar_offset: usize,
    end: usize,
) -> Option<EscapedBacktickParameterSyntax> {
    let next_offset = dollar_offset + '$'.len_utf8();
    let next = source.get(next_offset..end)?.chars().next()?;

    if matches!(next, '?' | '#' | '@' | '*' | '!' | '$' | '-') {
        return None;
    }
    if next.is_ascii_digit() {
        return Some(EscapedBacktickParameterSyntax::Simple {
            name: shuck_ast::Name::new(next.to_string()),
            expansion_len: "$0".len(),
        });
    }
    if next == '{' {
        let close_relative = source.get(next_offset + '{'.len_utf8()..end)?.find('}')?;
        let close_offset = next_offset + '{'.len_utf8() + close_relative;
        let inner = source.get(next_offset + '{'.len_utf8()..close_offset)?;
        let first = inner.chars().next()?;
        if matches!(first, '?' | '#' | '@' | '*' | '!' | '$' | '-') {
            return None;
        }
        let name_text = inner
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
            .collect::<String>();
        if name_text.is_empty() {
            return None;
        }
        let expansion_len = close_offset + '}'.len_utf8() - dollar_offset;
        if name_text.len() == inner.len() {
            return Some(EscapedBacktickParameterSyntax::Simple {
                name: shuck_ast::Name::new(name_text),
                expansion_len,
            });
        }
        let operator = &inner[name_text.len()..];
        if operator.starts_with(":+") || operator.starts_with('+') {
            return None;
        }
        return Some(EscapedBacktickParameterSyntax::ComplexUnsafe { expansion_len });
    }
    if next.is_ascii_alphabetic() || next == '_' {
        let mut cursor = next_offset;
        while cursor < end {
            let ch = source[cursor..]
                .chars()
                .next()
                .expect("cursor should remain on UTF-8 boundaries");
            if !(ch.is_ascii_alphanumeric() || ch == '_') {
                break;
            }
            cursor += ch.len_utf8();
        }
        let name = source.get(next_offset..cursor)?;
        return Some(EscapedBacktickParameterSyntax::Simple {
            name: shuck_ast::Name::new(name),
            expansion_len: cursor - dollar_offset,
        });
    }

    None
}

fn continued_line_chain_start(
    target: Position,
    containing_span: Span,
    source: &str,
) -> Option<Position> {
    let original_start = source[..target.offset]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let containing_line_start = source[..containing_span.start.offset]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let mut chain_start = containing_line_start;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_comment = false;
    let mut previous_char = None;
    let mut trailing_backslashes = 0usize;
    let mut index = containing_span.start.offset.saturating_add(1);

    while index < target.offset {
        if source[index..].starts_with("\r\n") {
            index += "\r\n".len();
            if !in_comment && !in_single_quote && trailing_backslashes % 2 == 1 {
                trailing_backslashes = 0;
                continue;
            }
            chain_start = index;
            in_comment = false;
            trailing_backslashes = 0;
            previous_char = Some('\n');
            continue;
        }

        let ch = source[index..]
            .chars()
            .next()
            .expect("index should remain on UTF-8 boundaries");
        let ch_len = ch.len_utf8();
        index += ch_len;

        if ch == '\n' {
            if !in_comment && !in_single_quote && trailing_backslashes % 2 == 1 {
                trailing_backslashes = 0;
                continue;
            }
            chain_start = index;
            in_comment = false;
            trailing_backslashes = 0;
            previous_char = Some('\n');
            continue;
        }

        if in_comment {
            trailing_backslashes = 0;
            continue;
        }

        if in_single_quote {
            if ch == '\'' {
                in_single_quote = false;
            }
            previous_char = Some(ch);
            trailing_backslashes = 0;
            continue;
        }

        let backslash_escaped = trailing_backslashes % 2 == 1;
        match ch {
            '\'' if !in_double_quote && !backslash_escaped => {
                in_single_quote = true;
                trailing_backslashes = 0;
            }
            '"' if !backslash_escaped => {
                in_double_quote = !in_double_quote;
                trailing_backslashes = 0;
            }
            '#' if !in_double_quote && backtick_shell_comment_can_start(previous_char) => {
                in_comment = true;
                trailing_backslashes = 0;
            }
            '\\' => {
                trailing_backslashes += 1;
            }
            _ => {
                trailing_backslashes = 0;
            }
        }

        previous_char = Some(ch);
    }

    (chain_start != original_start)
        .then(|| position_at_offset(source, chain_start))
        .flatten()
}

fn line_has_escaped_newline_continuation(line: &str) -> bool {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_comment = false;
    let mut previous_char = None;
    let mut trailing_backslashes = 0usize;

    for ch in line.chars() {
        if in_comment {
            trailing_backslashes = 0;
            continue;
        }

        if in_single_quote {
            if ch == '\'' {
                in_single_quote = false;
            }
            previous_char = Some(ch);
            trailing_backslashes = 0;
            continue;
        }

        let backslash_escaped = trailing_backslashes % 2 == 1;
        match ch {
            '\'' if !in_double_quote && !backslash_escaped => {
                in_single_quote = true;
                trailing_backslashes = 0;
            }
            '"' if !backslash_escaped => {
                in_double_quote = !in_double_quote;
                trailing_backslashes = 0;
            }
            '#' if !in_double_quote && backtick_shell_comment_can_start(previous_char) => {
                in_comment = true;
                trailing_backslashes = 0;
            }
            '\\' => {
                trailing_backslashes += 1;
            }
            _ => {
                trailing_backslashes = 0;
            }
        }

        previous_char = Some(ch);
    }

    !in_comment && !in_single_quote && trailing_backslashes % 2 == 1
}

fn shellcheck_collapsed_position(
    chain_start: Position,
    target: Position,
    source: &str,
) -> Position {
    let mut line = chain_start.line;
    let mut column = chain_start.column;
    let mut in_collapsed_continuation = false;
    let prefix = &source[chain_start.offset..target.offset];
    let mut index = 0usize;

    while index < prefix.len() {
        if prefix[index..].starts_with("\\\r\n") {
            index += "\\\r\n".len();
            in_collapsed_continuation = true;
            continue;
        }

        if prefix[index..].starts_with("\\\n") {
            index += "\\\n".len();
            in_collapsed_continuation = true;
            continue;
        }

        let ch = prefix[index..]
            .chars()
            .next()
            .expect("prefix iteration should stay on UTF-8 boundaries");
        if ch == '\n' {
            line += 1;
            column = 1;
            in_collapsed_continuation = false;
        } else if ch == '\t' && in_collapsed_continuation {
            column = ((column - 1) / 8 + 1) * 8 + 2;
        } else {
            column += 1;
        }
        index += ch.len_utf8();
    }

    Position {
        line,
        column,
        offset: target.offset,
    }
}

fn span_contains(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && outer.end.offset >= inner.end.offset
}

fn widen_dollar_paren_command_substitution_span(span: Span, source: &str) -> Option<Span> {
    let mut index = span.start.offset;
    let bytes = source.as_bytes();
    if bytes.get(index)? != &b'$' || bytes.get(index + 1)? != &b'(' {
        return None;
    }
    index += 2;

    let mut depth = 1usize;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while index < bytes.len() {
        let byte = bytes[index];

        if in_single_quote {
            if byte == b'\'' {
                in_single_quote = false;
            }
            index += 1;
            continue;
        }

        if in_double_quote {
            match byte {
                b'\\' => {
                    index = index.saturating_add(2);
                    continue;
                }
                b'"' => {
                    in_double_quote = false;
                    index += 1;
                    continue;
                }
                b'$' if bytes.get(index + 1) == Some(&b'(') => {
                    depth += 1;
                    index += 2;
                    continue;
                }
                b')' => {
                    depth = depth.saturating_sub(1);
                    index += 1;
                    if depth == 0 {
                        let start = position_at_offset(source, span.start.offset)?;
                        let end = position_at_offset(source, index)?;
                        return Some(Span::from_positions(start, end));
                    }
                    continue;
                }
                _ => {
                    index += 1;
                    continue;
                }
            }
        }

        match byte {
            b'\\' => {
                index = index.saturating_add(2);
            }
            b'\'' => {
                in_single_quote = true;
                index += 1;
            }
            b'"' => {
                in_double_quote = true;
                index += 1;
            }
            b'$' if bytes.get(index + 1) == Some(&b'(') => {
                depth += 1;
                index += 2;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                index += 1;
                if depth == 0 {
                    let start = position_at_offset(source, span.start.offset)?;
                    let end = position_at_offset(source, index)?;
                    return Some(Span::from_positions(start, end));
                }
            }
            _ => {
                index += 1;
            }
        }
    }

    None
}

fn widen_backtick_command_substitution_span(span: Span, source: &str) -> Option<Span> {
    let mut index = span.start.offset;
    let bytes = source.as_bytes();
    if bytes.get(index)? != &b'`' {
        return None;
    }
    index += 1;

    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index = index.saturating_add(2),
            b'`' => {
                index += 1;
                let start = position_at_offset(source, span.start.offset)?;
                let end = position_at_offset(source, index)?;
                return Some(Span::from_positions(start, end));
            }
            _ => index += 1,
        }
    }

    None
}

fn widen_all_elements_array_expansion_span(span: Span, source: &str) -> Option<Span> {
    let text = span.slice(source);
    if !text.contains("[@]") {
        return None;
    }

    let start_offset = span.start.offset.checked_sub(2)?;
    if source.as_bytes().get(start_offset..span.start.offset)? != b"${" {
        return None;
    }
    if offset_is_backslash_escaped(start_offset, source) {
        return None;
    }

    let start = position_at_offset(source, start_offset)?;
    let remainder = &source[start_offset..];
    let relative_end = remainder.find('}')?;
    let candidate = &remainder[..=relative_end];
    if !candidate_is_all_elements_array_expansion(candidate) {
        return None;
    }

    let end = position_at_offset(source, start_offset + candidate.len())?;
    Some(Span::from_positions(start, end))
}

fn widen_direct_all_elements_array_expansion_span(span: Span, source: &str) -> Option<Span> {
    let text = span.slice(source);
    if !text.contains("[@]") {
        return None;
    }

    let start_offset = span.start.offset.checked_sub(2)?;
    if source.as_bytes().get(start_offset..span.start.offset)? != b"${" {
        return None;
    }
    if offset_is_backslash_escaped(start_offset, source) {
        return None;
    }

    let start = position_at_offset(source, start_offset)?;
    let remainder = &source[start_offset..];
    let relative_end = remainder.find('}')?;
    let candidate = &remainder[..=relative_end];
    if !candidate_is_direct_all_elements_array_expansion(candidate) {
        return None;
    }

    let end = position_at_offset(source, start_offset + candidate.len())?;
    Some(Span::from_positions(start, end))
}

fn candidate_is_all_elements_array_expansion(candidate: &str) -> bool {
    let Some(inner) = candidate
        .strip_prefix("${")
        .and_then(|text| text.strip_suffix('}'))
    else {
        return false;
    };

    let (inner, indirect_like) = inner
        .strip_prefix('!')
        .map_or((inner, false), |stripped| (stripped, true));

    let Some(first) = inner.as_bytes().first().copied() else {
        return false;
    };

    if first == b'@' {
        return !indirect_like;
    }

    if !is_name_start(first) {
        return false;
    }

    let bytes = inner.as_bytes();
    let mut index = 1usize;
    while index < bytes.len() && is_name_continue(bytes[index]) {
        index += 1;
    }

    if inner[index..].starts_with("[@]") {
        return true;
    }

    indirect_like && inner[index..].starts_with('@')
}

fn candidate_is_direct_all_elements_array_expansion(candidate: &str) -> bool {
    let Some(mut inner) = candidate
        .strip_prefix("${")
        .and_then(|text| text.strip_suffix('}'))
    else {
        return false;
    };

    if let Some(stripped) = inner.strip_prefix('!') {
        inner = stripped;
    }

    let suffix = if let Some(stripped) = inner.strip_prefix('@') {
        stripped
    } else {
        let Some(first) = inner.as_bytes().first().copied() else {
            return false;
        };
        if !is_name_start(first) {
            return false;
        }

        let bytes = inner.as_bytes();
        let mut index = 1usize;
        while index < bytes.len() && is_name_continue(bytes[index]) {
            index += 1;
        }

        let Some(stripped) = inner[index..].strip_prefix("[@]") else {
            return false;
        };
        stripped
    };

    if suffix.starts_with('+') || suffix.starts_with(":+") {
        return false;
    }

    true
}

fn position_at_offset(source: &str, target_offset: usize) -> Option<Position> {
    if target_offset > source.len() {
        return None;
    }

    let mut position = Position::new();
    for ch in source[..target_offset].chars() {
        position.advance(ch);
    }
    Some(position)
}

fn collect_expansion_spans(parts: &[WordPartNode], spans: &mut Vec<Span>) {
    for part in parts {
        match &part.kind {
            WordPart::Literal(_) | WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => collect_expansion_spans(parts, spans),
            WordPart::Variable(name) if matches!(name.as_str(), "@" | "*") => spans.push(part.span),
            WordPart::Variable(_)
            | WordPart::ZshQualifiedGlob(_)
            | WordPart::CommandSubstitution { .. }
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
            | WordPart::ProcessSubstitution { .. }
            | WordPart::Transformation { .. } => spans.push(part.span),
        }
    }
}

fn collect_scalar_expansion_spans(
    parts: &[WordPartNode],
    quoted: bool,
    only_unquoted: bool,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::Literal(_) | WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_scalar_expansion_spans(parts, true, only_unquoted, spans)
            }
            WordPart::ZshQualifiedGlob(_) => {}
            WordPart::CommandSubstitution { .. } | WordPart::ProcessSubstitution { .. } => {}
            WordPart::Parameter(parameter) => {
                if parameter_is_scalar_like(parameter) && (!only_unquoted || !quoted) {
                    spans.push(part.span);
                }
            }
            WordPart::Variable(name) if matches!(name.as_str(), "@" | "*") => {}
            WordPart::Variable(_)
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayLength(_)
            | WordPart::Substring { .. }
            | WordPart::PrefixMatch { .. } => {
                if !only_unquoted || !quoted {
                    spans.push(part.span);
                }
            }
            WordPart::ParameterExpansion { reference, .. }
            | WordPart::IndirectExpansion { reference, .. }
            | WordPart::Transformation { reference, .. } => {
                if !reference.has_array_selector() && (!only_unquoted || !quoted) {
                    spans.push(part.span);
                }
            }
            WordPart::ArrayAccess(reference) => {
                if !reference.has_array_selector() && (!only_unquoted || !quoted) {
                    spans.push(part.span);
                }
            }
            WordPart::ArrayIndices(_) | WordPart::ArraySlice { .. } => {}
        }
    }
}

fn collect_use_replacement_spans(parts: &[WordPartNode], spans: &mut Vec<Span>) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => collect_use_replacement_spans(parts, spans),
            WordPart::Parameter(parameter) if parameter_uses_replacement_operator(parameter) => {
                spans.push(part.span);
            }
            WordPart::ParameterExpansion { operator, .. }
            | WordPart::IndirectExpansion {
                operator: Some(operator),
                ..
            } if matches!(operator, ParameterOp::UseReplacement) => spans.push(part.span),
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::ZshQualifiedGlob(_)
            | WordPart::CommandSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::Substring { .. }
            | WordPart::ArraySlice { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::Transformation { .. } => {}
            WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::IndirectExpansion { .. } => {}
        }
    }
}

fn collect_double_quoted_scalar_affix_state(
    parts: &[WordPartNode],
    saw_literal: &mut bool,
    saw_scalar_expansion: &mut bool,
    literal_span: &mut Option<Span>,
) -> bool {
    for part in parts {
        match &part.kind {
            WordPart::Literal(_) | WordPart::SingleQuoted { .. } => {
                *saw_literal = true;
                if literal_span.is_none() {
                    *literal_span = Some(part.span);
                }
            }
            WordPart::DoubleQuoted { parts, .. } => {
                if !collect_double_quoted_scalar_affix_state(
                    parts,
                    saw_literal,
                    saw_scalar_expansion,
                    literal_span,
                ) {
                    return false;
                }
            }
            WordPart::Variable(_)
            | WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::Substring { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::Transformation { .. } => {
                *saw_scalar_expansion = true;
            }
            WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::ArraySlice { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::CommandSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::ZshQualifiedGlob(_) => {
                return false;
            }
        }
    }

    true
}

fn collect_double_quoted_scalar_only_expansion_spans(
    parts: &[WordPartNode],
    inside_double_quotes: bool,
    spans: &mut Vec<Span>,
) -> bool {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                if !collect_double_quoted_scalar_only_expansion_spans(parts, true, spans) {
                    return false;
                }
            }
            WordPart::Variable(_)
            | WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::Substring { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::Transformation { .. } => {
                if !inside_double_quotes {
                    return false;
                }
                spans.push(part.span);
            }
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::ArraySlice { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::CommandSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::ZshQualifiedGlob(_) => {
                return false;
            }
        }
    }

    true
}

fn normalize_shell_quoting_segment_span(word: &Word, span: Span, source: &str) -> Span {
    let mut start = span.start;
    let mut end = span.end;
    let text = span.slice(source);
    if word.is_fully_double_quoted() {
        if span.start.offset == word.span.start.offset && text.starts_with('"') {
            start = start.advanced_by("\"");
        }
        if span.end.offset == word.span.end.offset && text.ends_with('"') {
            end = span.start.advanced_by(&text[..text.len() - 1]);
        }
    }

    let normalized = Span::from_positions(start, end);
    let normalized_text = normalized.slice(source);
    if normalized_text.ends_with('\\')
        && let Some(next) = source
            .get(normalized.end.offset..)
            .and_then(|tail| tail.chars().next())
        && matches!(next, '"' | '\'')
    {
        let quote = if next == '"' { "\"" } else { "'" };
        return Span::from_positions(normalized.start, normalized.end.advanced_by(quote));
    }

    normalized
}

fn text_contains_shell_quoting_literals(text: &str) -> bool {
    if text.contains(['"', '\'']) {
        return true;
    }

    let chars = text.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < chars.len() {
        if chars[index] != '\\' {
            index += 1;
            continue;
        }

        let mut end = index + 1;
        while end < chars.len() && chars[end] == '\\' {
            end += 1;
        }
        if chars.get(end).is_some_and(|next| {
            matches!(next, '"' | '\'') || (next.is_whitespace() && !matches!(next, '\n' | '\r'))
        }) {
            return true;
        }

        index = end;
    }

    false
}

fn text_position_is_escaped(text: &str, offset: usize) -> bool {
    let bytes = text.as_bytes();
    let mut cursor = offset;
    let mut backslashes = 0usize;
    while cursor > 0 {
        cursor -= 1;
        if bytes[cursor] != b'\\' {
            break;
        }
        backslashes += 1;
    }

    backslashes % 2 == 1
}
fn literal_part_is_parameter_operator_tail(
    parts: &[WordPartNode],
    index: usize,
    source: &str,
) -> bool {
    let Some(previous) = index.checked_sub(1).and_then(|index| parts.get(index)) else {
        return false;
    };
    if !matches!(
        previous.kind,
        WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::IndirectExpansion { .. }
    ) {
        return false;
    }

    let text = parts[index].span.slice(source);
    text.ends_with('}') && (text.starts_with('/') || text.starts_with('%') || text.starts_with('#'))
}

fn collect_literal_scan_exclusions(parts: &[WordPartNode], excluded: &mut Vec<Span>) {
    for part in parts {
        match &part.kind {
            WordPart::Literal(_) => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_literal_scan_exclusions(parts, excluded);
            }
            WordPart::CommandSubstitution { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::SingleQuoted { .. }
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
            | WordPart::ZshQualifiedGlob(_) => excluded.push(part.span),
        }
    }
}

fn collect_unquoted_glob_pattern_spans(
    parts: &[WordPartNode],
    source: &str,
    in_double_quotes: bool,
    spans: &mut Vec<Span>,
) {
    let mut literal_run_start = None::<usize>;
    let mut literal_run_end = None::<usize>;

    let flush_literal_run = |literal_run_start: &mut Option<usize>,
                             literal_run_end: &mut Option<usize>,
                             spans: &mut Vec<Span>| {
        let Some(start_index) = literal_run_start.take() else {
            return;
        };
        let Some(end_index) = literal_run_end.take() else {
            return;
        };
        let start = parts[start_index].span.start;
        let end = parts[end_index].span.end;
        let combined_span = Span::from_positions(start, end);
        spans.extend(literal_glob_pattern_spans(combined_span, source));
    };

    for (index, part) in parts.iter().enumerate() {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                flush_literal_run(&mut literal_run_start, &mut literal_run_end, spans);
                collect_unquoted_glob_pattern_spans(parts, source, true, spans)
            }
            WordPart::Literal(_)
                if !in_double_quotes
                    && !literal_part_is_parameter_operator_tail(parts, index, source) =>
            {
                literal_run_start.get_or_insert(index);
                literal_run_end = Some(index);
            }
            WordPart::Literal(_)
            | WordPart::CommandSubstitution { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::SingleQuoted { .. }
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
            | WordPart::ZshQualifiedGlob(_) => {
                flush_literal_run(&mut literal_run_start, &mut literal_run_end, spans);
            }
        }
    }

    flush_literal_run(&mut literal_run_start, &mut literal_run_end, spans);
}

fn parts_have_unquoted_brace_expansion(
    parts: &[WordPartNode],
    source: &str,
    in_double_quotes: bool,
) -> bool {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                if parts_have_unquoted_brace_expansion(parts, source, true) {
                    return true;
                }
            }
            WordPart::Literal(_) if !in_double_quotes => {
                if literal_contains_brace_expansion(part.span.slice(source)) {
                    return true;
                }
            }
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::CommandSubstitution { .. }
            | WordPart::ProcessSubstitution { .. }
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
        }
    }

    false
}

fn literal_contains_brace_expansion(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
            continue;
        }

        if bytes[index] != b'{' {
            index += 1;
            continue;
        }

        let mut depth = 1usize;
        let mut saw_comma = false;
        let mut saw_range = false;
        let mut cursor = index + 1;
        while cursor < bytes.len() {
            if bytes[cursor] == b'\\' {
                cursor = (cursor + 2).min(bytes.len());
                continue;
            }

            match bytes[cursor] {
                b'{' => depth += 1,
                b'}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        if saw_comma || saw_range {
                            return true;
                        }
                        break;
                    }
                }
                b',' if depth == 1 => saw_comma = true,
                b'.' if depth == 1
                    && cursor + 1 < bytes.len()
                    && bytes[cursor + 1] == b'.'
                    && !byte_is_backslash_escaped(bytes, cursor)
                    && !byte_is_backslash_escaped(bytes, cursor + 1) =>
                {
                    saw_range = true;
                    cursor += 1;
                }
                _ => {}
            }
            cursor += 1;
        }

        index += 1;
    }

    false
}

fn literal_glob_pattern_spans(span: Span, source: &str) -> Vec<Span> {
    let text = span.slice(source);
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
            continue;
        }

        match bytes[index] {
            b'*' | b'?' => {
                spans.push(span_within_literal(span, source, index, index + 1));
                index += 1;
            }
            b'[' => {
                let mut end = index + 1;
                while end < bytes.len() {
                    if let Some(named_end) = bracket_glob_named_class_end(bytes, end, bytes.len()) {
                        end = named_end;
                        continue;
                    }
                    if bytes[end] == b'\\' {
                        end = (end + 2).min(bytes.len());
                        continue;
                    }
                    if bytes[end] == b']' {
                        break;
                    }
                    end += 1;
                }
                if end < bytes.len() {
                    spans.push(span_within_literal(span, source, index, end + 1));
                    index = end + 1;
                } else {
                    index += 1;
                }
            }
            _ => index += 1,
        }
    }

    spans
}

pub(crate) fn suspicious_bracket_glob_text(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() < 3 || bytes[0] != b'[' || *bytes.last().unwrap_or(&b'\0') != b']' {
        return false;
    }
    if bracket_glob_is_named_class_without_outer_brackets(bytes) {
        return false;
    }

    let mut seen = std::collections::HashSet::new();
    let start = usize::from(matches!(bytes[1], b'!' | b'^')) + 1;
    let mut index = start;
    let end = bytes.len() - 1;

    while index < end {
        if let Some(next) = bracket_glob_named_class_end(bytes, index, bytes.len()) {
            index = next;
            continue;
        }
        if hyphen_is_range_separator(bytes, index, start, end) {
            index += 1;
            continue;
        }

        if bytes[index] == b'\\' {
            if index + 1 >= end {
                break;
            }
            let Some(escaped) = text[index + 1..end].chars().next() else {
                break;
            };
            if !seen.insert(escaped) {
                return true;
            }
            index += 1 + escaped.len_utf8();
            continue;
        }

        let Some(character) = text[index..end].chars().next() else {
            break;
        };
        if !seen.insert(character) {
            return true;
        }
        index += character.len_utf8();
    }

    false
}

fn bracket_glob_is_named_class_without_outer_brackets(bytes: &[u8]) -> bool {
    if bytes.len() < 5 {
        return false;
    }

    let kind = bytes[1];
    if !matches!(kind, b':' | b'.' | b'=') {
        return false;
    }

    bytes[bytes.len() - 2] == kind
}

fn bracket_glob_named_class_end(bytes: &[u8], start: usize, limit: usize) -> Option<usize> {
    if start + 3 >= limit || bytes[start] != b'[' {
        return None;
    }

    let kind = bytes[start + 1];
    if !matches!(kind, b':' | b'.' | b'=') {
        return None;
    }

    let mut index = start + 2;
    while index + 1 < limit {
        if bytes[index] == b'\\' {
            index = (index + 2).min(limit);
            continue;
        }

        if bytes[index] == kind && bytes[index + 1] == b']' {
            return Some(index + 2);
        }
        index += 1;
    }

    None
}

fn hyphen_is_range_separator(bytes: &[u8], index: usize, start: usize, end: usize) -> bool {
    if bytes[index] != b'-' || index == start || index + 1 >= end {
        return false;
    }

    if bracket_glob_named_class_end(bytes, index + 1, bytes.len()).is_some() {
        return false;
    }

    true
}

fn span_within_literal(span: Span, source: &str, start: usize, end: usize) -> Span {
    let start_pos = span
        .start
        .advanced_by(&source[span.start.offset..span.start.offset + start]);
    let end_pos = span
        .start
        .advanced_by(&source[span.start.offset..span.start.offset + end]);
    Span::from_positions(start_pos, end_pos)
}

fn scan_span_excluding(span: Span, excluded: &[Span], source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_scan_span_excluding(span, excluded, source, &mut spans);
    spans
}

fn collect_scan_span_excluding(span: Span, excluded: &[Span], source: &str, spans: &mut Vec<Span>) {
    if excluded.is_empty() {
        spans.push(span);
        return;
    }

    let mut cursor = span.start.offset;
    for excluded_span in excluded.iter().copied().filter(|excluded_span| {
        excluded_span.end.offset > span.start.offset && excluded_span.start.offset < span.end.offset
    }) {
        let segment_end = excluded_span.start.offset.min(span.end.offset);
        if cursor < segment_end {
            spans.push(scan_span_segment(span, cursor, segment_end, source));
        }
        cursor = cursor.max(excluded_span.end.offset).min(span.end.offset);
    }

    if cursor < span.end.offset {
        spans.push(scan_span_segment(span, cursor, span.end.offset, source));
    }
}

fn merge_adjacent_spans(spans: Vec<Span>, source: &str) -> Vec<Span> {
    let mut merged: Vec<Span> = Vec::new();

    for span in spans {
        if let Some(previous) = merged.last_mut()
            && spans_share_literal_run(*previous, span, source)
        {
            *previous = Span::from_positions(previous.start, span.end);
            continue;
        }

        merged.push(span);
    }

    merged
}

fn shell_quoting_literal_run_span(
    word: &Word,
    span: Span,
    excluded: &[Span],
    source: &str,
) -> Span {
    let start = excluded
        .iter()
        .copied()
        .filter(|excluded_span| excluded_span.start.offset < span.start.offset)
        .map(|excluded_span| excluded_span.end)
        .max_by_key(|position| position.offset)
        .unwrap_or(word.span.start);
    let end = excluded
        .iter()
        .copied()
        .filter(|excluded_span| excluded_span.start.offset > start.offset)
        .map(|excluded_span| excluded_span.start)
        .min_by_key(|position| position.offset)
        .unwrap_or(word.span.end);

    normalize_shell_quoting_segment_span(word, Span::from_positions(start, end), source)
}

fn word_shell_quoting_segment_span_in_source(
    word: &Word,
    text: &str,
    start: usize,
    end: usize,
) -> Option<Span> {
    let segment = &text[start..end];
    if !text_contains_shell_quoting_literals(segment) {
        return None;
    }

    let trimmed_start = if let Some(anchor) = first_shell_quoting_escape_anchor(segment) {
        segment[..anchor]
            .rfind('\'')
            .map_or(start, |quote| start + quote + 1)
    } else {
        start
    };

    Some(Span::from_positions(
        word.span.start.advanced_by(&text[..trimmed_start]),
        word.span.start.advanced_by(&text[..end]),
    ))
}

fn first_shell_quoting_escape_anchor(text: &str) -> Option<usize> {
    let chars = text.char_indices().collect::<Vec<_>>();
    for (index, (offset, ch)) in chars.iter().copied().enumerate() {
        if ch != '\\' {
            continue;
        }
        if let Some((_, next)) = chars.get(index + 1).copied()
            && (matches!(next, '"' | '\'') || next.is_whitespace())
        {
            return Some(offset);
        }
    }

    first_shell_quoting_anchor(text)
}

fn first_shell_quoting_anchor(text: &str) -> Option<usize> {
    let chars = text.char_indices().collect::<Vec<_>>();
    for (index, (offset, ch)) in chars.iter().copied().enumerate() {
        if matches!(ch, '"' | '\'') {
            return Some(offset);
        }
        if ch != '\\' {
            continue;
        }
        if let Some((_, next)) = chars.get(index + 1).copied()
            && (matches!(next, '"' | '\'') || next.is_whitespace())
        {
            return Some(offset);
        }
    }

    None
}

fn shell_quoting_expansion_len(text: &str) -> usize {
    if text.starts_with('`') {
        return closing_backtick_offset(text).unwrap_or(1);
    }
    if !text.starts_with('$') {
        return 1;
    }

    if text.starts_with("${") {
        return braced_expansion_len(text).unwrap_or(2);
    }
    if text.starts_with("$(") {
        return paren_expansion_len(text).unwrap_or(2);
    }

    let bytes = text.as_bytes();
    let Some(&next) = bytes.get(1) else {
        return 1;
    };
    if (next as char).is_ascii_alphabetic() || next == b'_' {
        let mut end = 2usize;
        while let Some(byte) = bytes.get(end) {
            let ch = *byte as char;
            if ch.is_ascii_alphanumeric() || ch == '_' {
                end += 1;
                continue;
            }
            break;
        }
        return end;
    }
    if (next as char).is_ascii_digit() || b"@*#?$!-".contains(&next) {
        return 2;
    }

    1
}

fn closing_backtick_offset(text: &str) -> Option<usize> {
    let mut chars = text.char_indices();
    chars.next()?;
    for (offset, ch) in chars {
        if ch == '`' && !text_position_is_escaped(text, offset) {
            return Some(offset + 1);
        }
    }

    None
}

fn braced_expansion_len(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, ch) in text.char_indices() {
        match ch {
            '$' if offset == 0 => {}
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(offset + 1);
                }
            }
            _ => {}
        }
    }

    None
}

fn paren_expansion_len(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, ch) in text.char_indices() {
        match ch {
            '$' if offset == 0 => {}
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(offset + 1);
                }
            }
            _ => {}
        }
    }

    None
}

fn spans_share_literal_run(previous: Span, next: Span, source: &str) -> bool {
    if previous.end.offset >= next.start.offset {
        return true;
    }

    let gap = &source[previous.end.offset..next.start.offset];
    !gap.contains('$') && !gap.contains('`')
}

fn scan_span_segment(span: Span, start: usize, end: usize, source: &str) -> Span {
    let segment_start = span.start.advanced_by(&source[span.start.offset..start]);
    let segment_end = span.start.advanced_by(&source[span.start.offset..end]);
    Span::from_positions(segment_start, segment_end)
}

fn pattern_extglob_span(pattern: &Pattern, source: &str) -> Option<Span> {
    for part in &pattern.parts {
        match &part.kind {
            PatternPart::Group { patterns, .. } => {
                return Some(part.span).or_else(|| {
                    patterns
                        .iter()
                        .find_map(|pattern| pattern_extglob_span(pattern, source))
                });
            }
            PatternPart::Word(word) => {
                if let Some(span) = word_extglob_span(word, source) {
                    return Some(span);
                }
            }
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }

    None
}

fn pattern_array_subscript_span(pattern: &Pattern, source: &str) -> Option<Span> {
    for part in &pattern.parts {
        match &part.kind {
            PatternPart::Group { patterns, .. } => {
                if let Some(span) = patterns
                    .iter()
                    .find_map(|pattern| pattern_array_subscript_span(pattern, source))
                {
                    return Some(span);
                }
            }
            PatternPart::Word(word) => {
                if let Some(span) = word_array_subscript_span(word, source) {
                    return Some(span);
                }
            }
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }

    None
}

fn word_array_subscript_span_from_parts(parts: &[WordPartNode], source: &str) -> Option<Span> {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                if let Some(span) = word_array_subscript_span_from_parts(parts, source) {
                    return Some(span);
                }
            }
            WordPart::Literal(_) => {
                if text_has_variable_subscript(part.span.slice(source)) {
                    return Some(part.span);
                }
            }
            WordPart::Parameter(parameter) => {
                if let Some(span) = parameter_array_subscript_span(parameter) {
                    return Some(span);
                }
            }
            WordPart::ParameterExpansion { reference, .. }
            | WordPart::Length(reference)
            | WordPart::ArrayAccess(reference)
            | WordPart::ArrayLength(reference)
            | WordPart::ArrayIndices(reference)
            | WordPart::Substring { reference, .. }
            | WordPart::ArraySlice { reference, .. }
            | WordPart::IndirectExpansion { reference, .. }
            | WordPart::Transformation { reference, .. } => {
                if let Some(span) = var_ref_subscript_span(reference) {
                    return Some(span);
                }
            }
            WordPart::ZshQualifiedGlob(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::CommandSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::ProcessSubstitution { .. } => {}
        }
    }

    None
}

fn collect_unbraced_variable_before_bracket_spans(
    parts: &[WordPartNode],
    source: &str,
    spans: &mut Vec<Span>,
) {
    let mut pending_variable = None;

    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts: inner, .. } => {
                collect_unbraced_variable_before_bracket_spans(inner, source, spans);
                pending_variable = None;
            }
            WordPart::SingleQuoted { .. }
            | WordPart::CommandSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::ProcessSubstitution { .. } => {
                pending_variable = None;
            }
            WordPart::Variable(name)
                if is_named_shell_variable(name.as_str())
                    && !variable_part_uses_braces(part, source) =>
            {
                pending_variable = Some(unbraced_variable_dollar_span(part, source));
            }
            WordPart::Literal(text) => {
                if text.as_str(source, part.span).starts_with('[')
                    && let Some(variable_span) = pending_variable
                {
                    spans.push(variable_span);
                }
                pending_variable = None;
            }
            WordPart::Variable(_)
            | WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::Substring { .. }
            | WordPart::ArraySlice { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::Transformation { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::ZshQualifiedGlob(_) => {
                pending_variable = None;
            }
        }
    }
}

fn is_named_shell_variable(name: &str) -> bool {
    let bytes = name.as_bytes();
    let Some((&first, rest)) = bytes.split_first() else {
        return false;
    };

    is_name_start(first) && rest.iter().copied().all(is_name_continue)
}

fn unbraced_variable_dollar_span(part: &WordPartNode, source: &str) -> Span {
    let raw = part.span.slice(source);
    let dollar_offset = raw.find('$').unwrap_or(0);
    Span::at(part.span.start.advanced_by(&raw[..dollar_offset]))
}

fn variable_part_uses_braces(part: &WordPartNode, source: &str) -> bool {
    let raw = part.span.slice(source);
    raw.find('$')
        .and_then(|offset| raw.as_bytes().get(offset + 1))
        .is_some_and(|next| *next == b'{')
}

fn parameter_array_subscript_span(parameter: &ParameterExpansion) -> Option<Span> {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Indirect { reference, .. }
            | BourneParameterExpansion::Slice { reference, .. }
            | BourneParameterExpansion::Operation { reference, .. }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                var_ref_subscript_span(reference)
            }
            BourneParameterExpansion::PrefixMatch { .. } => None,
        },
        ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
            ZshExpansionTarget::Reference(reference) => var_ref_subscript_span(reference),
            ZshExpansionTarget::Nested(parameter) => parameter_array_subscript_span(parameter),
            ZshExpansionTarget::Word(_) | ZshExpansionTarget::Empty => None,
        },
    }
}

fn var_ref_subscript_span(reference: &VarRef) -> Option<Span> {
    reference
        .subscript
        .as_ref()
        .filter(|subscript| subscript.selector().is_none())
        .map(|_| reference.span)
}

fn word_surface_bytes(word: &Word, source: &str) -> Option<(Vec<u8>, Vec<Option<usize>>)> {
    if word.has_quoted_parts() {
        return None;
    }

    let word_start = word.span.start.offset;
    let mut surface = Vec::new();
    let mut source_offsets = Vec::new();

    for part in &word.parts {
        match &part.kind {
            WordPart::Literal(_) => {
                let part_text = part.span.slice(source);
                let relative_start = part.span.start.offset.checked_sub(word_start)?;
                for (index, byte) in part_text.as_bytes().iter().copied().enumerate() {
                    surface.push(byte);
                    source_offsets.push(Some(relative_start + index));
                }
            }
            WordPart::DoubleQuoted { .. } | WordPart::SingleQuoted { .. } => return None,
            WordPart::ZshQualifiedGlob(_)
            | WordPart::Variable(_)
            | WordPart::CommandSubstitution { .. }
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
            | WordPart::ProcessSubstitution { .. }
            | WordPart::Transformation { .. } => {
                surface.push(b'_');
                source_offsets.push(None);
            }
        }
    }

    Some((surface, source_offsets))
}

fn word_extglob_span_from_literal_parts(parts: &[WordPartNode], source: &str) -> Option<Span> {
    for part in parts {
        if matches!(part.kind, WordPart::Literal(_))
            && find_extglob_bounds(part.span.slice(source).as_bytes()).is_some()
        {
            return Some(part.span);
        }
    }

    None
}

fn word_exactly_one_extglob_span_from_literal_parts(
    parts: &[WordPartNode],
    source: &str,
) -> Option<Span> {
    for part in parts {
        if matches!(part.kind, WordPart::Literal(_))
            && find_exactly_one_extglob_bounds(part.span.slice(source).as_bytes()).is_some()
        {
            return Some(part.span);
        }
    }

    None
}

fn word_caret_negated_bracket_spans_from_literal_parts(
    parts: &[WordPartNode],
    source: &str,
) -> Vec<Span> {
    parts
        .iter()
        .filter(|part| matches!(part.kind, WordPart::Literal(_)))
        .flat_map(|part| {
            let text = part.span.slice(source);
            find_caret_negated_bracket_bounds(text.as_bytes())
                .into_iter()
                .map(move |(start, end)| {
                    Span::from_positions(
                        part.span.start.advanced_by(&text[..start]),
                        part.span.start.advanced_by(&text[..end + 1]),
                    )
                })
        })
        .collect()
}

fn word_surface_span_from_bounds(
    word: &Word,
    source: &str,
    source_offsets: &[Option<usize>],
    start: usize,
    end: usize,
) -> Option<Span> {
    let start_offset = source_offsets.get(start).copied().flatten()?;
    let end_offset = source_offsets.get(end).copied().flatten()?;
    let word_text = word.span.slice(source);

    Some(Span::from_positions(
        word.span.start.advanced_by(&word_text[..start_offset]),
        word.span.start.advanced_by(&word_text[..end_offset + 1]),
    ))
}

fn word_has_only_literal_parts(parts: &[WordPartNode]) -> bool {
    parts
        .iter()
        .all(|part| matches!(part.kind, WordPart::Literal(_)))
}

fn pattern_exactly_one_extglob_span(pattern: &Pattern, source: &str) -> Option<Span> {
    for part in &pattern.parts {
        match &part.kind {
            PatternPart::Group { kind, patterns } => {
                if *kind == PatternGroupKind::ExactlyOne {
                    return Some(part.span);
                }

                if let Some(span) = patterns
                    .iter()
                    .find_map(|pattern| pattern_exactly_one_extglob_span(pattern, source))
                {
                    return Some(span);
                }
            }
            PatternPart::Word(word) => {
                if let Some(span) = word_exactly_one_extglob_span(word, source) {
                    return Some(span);
                }
            }
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }

    None
}

fn text_has_variable_subscript(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'$' || byte_is_backslash_escaped(bytes, index) {
            index += 1;
            continue;
        }

        let next = index + 1;
        if next >= bytes.len() {
            break;
        }

        if bytes[next] == b'{' {
            let mut cursor = next + 1;
            while cursor < bytes.len() && bytes[cursor] != b'}' {
                if bytes[cursor] == b'['
                    && bytes[cursor + 1..].contains(&b']')
                    && bytes[cursor + 1..].contains(&b'}')
                {
                    return true;
                }
                cursor += 1;
            }
            index = cursor.saturating_add(1);
            continue;
        }

        if !is_name_start(bytes[next]) {
            index += 1;
            continue;
        }

        let mut cursor = next + 1;
        while cursor < bytes.len() && is_name_continue(bytes[cursor]) {
            cursor += 1;
        }

        if cursor < bytes.len() && bytes[cursor] == b'[' && bytes[cursor + 1..].contains(&b']') {
            return true;
        }

        index = cursor;
    }

    false
}

fn find_parenthesized_alternation_bounds(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'(' || byte_is_backslash_escaped(bytes, index) {
            index += 1;
            continue;
        }

        let Some(close) = matching_group_end(bytes, index) else {
            index += 1;
            continue;
        };

        if bytes[index + 1..close]
            .iter()
            .enumerate()
            .any(|(offset, byte)| {
                *byte == b'|' && !byte_is_backslash_escaped(bytes, index + 1 + offset)
            })
        {
            return Some((index, close));
        }

        index = close + 1;
    }

    None
}

fn find_extglob_bounds(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut index = 0usize;
    while index + 1 < bytes.len() {
        if !is_extglob_operator(bytes[index])
            || bytes[index + 1] != b'('
            || byte_is_backslash_escaped(bytes, index)
        {
            index += 1;
            continue;
        }

        if let Some(close) = matching_group_end(bytes, index + 1) {
            return Some((index, close));
        }

        index += 1;
    }

    find_parenthesized_alternation_bounds(bytes)
}

fn find_exactly_one_extglob_bounds(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut index = 0usize;
    while index + 1 < bytes.len() {
        if bytes[index] != b'@'
            || bytes[index + 1] != b'('
            || byte_is_backslash_escaped(bytes, index)
        {
            index += 1;
            continue;
        }

        if let Some(close) = matching_group_end(bytes, index + 1) {
            return Some((index, close));
        }

        index += 1;
    }

    None
}

fn find_caret_negated_bracket_bounds(bytes: &[u8]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut index = 0usize;

    while index + 1 < bytes.len() {
        if bytes[index] != b'['
            || byte_is_backslash_escaped(bytes, index)
            || bytes[index + 1] != b'^'
            || byte_is_backslash_escaped(bytes, index + 1)
        {
            index += 1;
            continue;
        }

        let mut close = index + 2;
        while close < bytes.len() {
            if bytes[close] == b']' && !byte_is_backslash_escaped(bytes, close) {
                spans.push((index, close));
                index = close + 1;
                break;
            }
            close += 1;
        }

        if close >= bytes.len() {
            break;
        }
    }

    spans
}

fn matching_group_end(bytes: &[u8], open_index: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut cursor = open_index + 1;

    while cursor < bytes.len() {
        if byte_is_backslash_escaped(bytes, cursor) {
            cursor += 1;
            continue;
        }

        match bytes[cursor] {
            b'(' => {
                depth += 1;
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(cursor);
                }
            }
            _ => {}
        }

        cursor += 1;
    }

    None
}

fn byte_is_backslash_escaped(bytes: &[u8], index: usize) -> bool {
    let mut cursor = index;
    let mut backslashes = 0usize;

    while cursor > 0 && bytes[cursor - 1] == b'\\' {
        backslashes += 1;
        cursor -= 1;
    }

    backslashes % 2 == 1
}

fn is_extglob_operator(byte: u8) -> bool {
    matches!(byte, b'@' | b'?' | b'+' | b'*' | b'!')
}

fn is_name_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

fn is_name_continue(byte: u8) -> bool {
    is_name_start(byte) || byte.is_ascii_digit()
}

fn parameter_is_array_like(parameter: &ParameterExpansion) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference } => reference.has_array_selector(),
            BourneParameterExpansion::Indices { .. } => true,
            BourneParameterExpansion::Slice { reference, .. } => reference.has_array_selector(),
            BourneParameterExpansion::Operation {
                reference,
                operator,
                ..
            } => !matches!(operator, ParameterOp::UseReplacement) && reference.has_array_selector(),
            BourneParameterExpansion::Transformation { reference, .. } => {
                reference.has_array_selector()
            }
            _ => false,
        },
        ParameterExpansionSyntax::Zsh(_) => false,
    }
}

fn parameter_might_use_all_elements_array_expansion(
    parameter: &ParameterExpansion,
    span: Span,
    source: &str,
) -> bool {
    if !span.slice(source).contains('@') {
        return false;
    }

    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Length { .. } | BourneParameterExpansion::Indirect { .. } => {
                false
            }
            BourneParameterExpansion::PrefixMatch { kind, .. } => {
                matches!(kind, PrefixMatchKind::At)
            }
            _ => true,
        },
        ParameterExpansionSyntax::Zsh(_) => true,
    }
}

fn parameter_is_scalar_like(parameter: &ParameterExpansion) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference } => !reference.has_array_selector(),
            BourneParameterExpansion::Length { .. }
            | BourneParameterExpansion::PrefixMatch { .. } => true,
            BourneParameterExpansion::Indirect { reference, .. }
            | BourneParameterExpansion::Operation { reference, .. }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                !reference.has_array_selector()
            }
            BourneParameterExpansion::Indices { .. } => false,
            BourneParameterExpansion::Slice { reference, .. } => !reference.has_array_selector(),
        },
        ParameterExpansionSyntax::Zsh(_) => true,
    }
}

fn parameter_uses_replacement_operator(parameter: &ParameterExpansion) -> bool {
    let ParameterExpansionSyntax::Bourne(syntax) = &parameter.syntax else {
        return false;
    };

    match syntax {
        BourneParameterExpansion::Indirect {
            operator: Some(operator),
            ..
        }
        | BourneParameterExpansion::Operation { operator, .. } => {
            matches!(operator, ParameterOp::UseReplacement)
        }
        BourneParameterExpansion::Access { .. }
        | BourneParameterExpansion::Length { .. }
        | BourneParameterExpansion::Indices { .. }
        | BourneParameterExpansion::PrefixMatch { .. }
        | BourneParameterExpansion::Slice { .. }
        | BourneParameterExpansion::Transformation { .. }
        | BourneParameterExpansion::Indirect { operator: None, .. } => false,
    }
}

fn part_uses_star_splat(part: &WordPart) -> bool {
    match part {
        WordPart::Variable(name) => name.as_str() == "*",
        WordPart::ArrayAccess(reference) => var_ref_uses_star_splat(reference),
        WordPart::Parameter(parameter) => parameter_uses_star_splat(parameter),
        WordPart::ParameterExpansion { reference, .. }
        | WordPart::IndirectExpansion { reference, .. }
        | WordPart::Transformation { reference, .. } => var_ref_uses_star_splat(reference),
        _ => false,
    }
}

fn part_uses_all_elements_array_slice(part: &WordPart) -> bool {
    match part {
        WordPart::ArraySlice { reference, .. } => var_ref_uses_all_elements_at_splat(reference),
        WordPart::Parameter(parameter) => parameter_uses_all_elements_array_slice(parameter),
        _ => false,
    }
}

fn part_uses_positional_at_splat(part: &WordPart) -> bool {
    match part {
        WordPart::Variable(name) => name.as_str() == "@",
        WordPart::ArrayAccess(reference) => var_ref_uses_positional_at_splat(reference),
        WordPart::Parameter(parameter) => parameter_uses_positional_at_splat(parameter),
        _ => false,
    }
}

fn part_uses_unquoted_all_elements_array_expansion(part: &WordPart) -> bool {
    match part {
        WordPart::Variable(name) => name.as_str() == "@",
        WordPart::ArrayAccess(reference) | WordPart::ArrayIndices(reference) => {
            var_ref_uses_all_elements_at_splat(reference)
        }
        WordPart::ArraySlice { reference, .. } => var_ref_uses_all_elements_at_splat(reference),
        WordPart::Parameter(parameter) => {
            parameter_uses_unquoted_all_elements_array_expansion(parameter)
        }
        _ => false,
    }
}

fn part_uses_direct_all_elements_array_expansion(part: &WordPart) -> bool {
    match part {
        WordPart::Variable(name) => name.as_str() == "@",
        WordPart::ArrayAccess(reference) | WordPart::ArrayIndices(reference) => {
            var_ref_uses_all_elements_at_splat(reference)
        }
        WordPart::ArraySlice { reference, .. } => var_ref_uses_all_elements_at_splat(reference),
        WordPart::Parameter(parameter) => {
            parameter_uses_direct_all_elements_array_expansion(parameter)
        }
        _ => false,
    }
}

fn part_is_pure_positional_at_splat(part: &WordPart) -> bool {
    match part {
        WordPart::Variable(name) => name.as_str() == "@",
        WordPart::ArrayAccess(reference) => var_ref_uses_positional_at_splat(reference),
        WordPart::Parameter(parameter) => parameter_is_pure_positional_at_splat(parameter),
        _ => false,
    }
}

fn part_uses_assign_default_operator(part: &WordPart) -> bool {
    match part {
        WordPart::Parameter(parameter) => parameter_uses_assign_default_operator(parameter),
        WordPart::ParameterExpansion { operator, .. }
        | WordPart::IndirectExpansion {
            operator: Some(operator),
            ..
        } => matches!(operator, ParameterOp::AssignDefault),
        _ => false,
    }
}

fn var_ref_uses_star_splat(reference: &VarRef) -> bool {
    reference.name.as_str() == "*"
        || matches!(
            reference
                .subscript
                .as_ref()
                .and_then(|subscript| subscript.selector()),
            Some(SubscriptSelector::Star)
        )
}

fn var_ref_uses_all_elements_at_splat(reference: &VarRef) -> bool {
    reference.name.as_str() == "@"
        || matches!(
            reference
                .subscript
                .as_ref()
                .and_then(|subscript| subscript.selector()),
            Some(SubscriptSelector::At)
        )
}

fn parameter_uses_all_elements_array_slice(parameter: &ParameterExpansion) -> bool {
    let ParameterExpansionSyntax::Bourne(syntax) = &parameter.syntax else {
        return false;
    };

    matches!(
        syntax,
        BourneParameterExpansion::Slice { reference, .. }
            if var_ref_uses_all_elements_at_splat(reference)
    )
}

fn parameter_uses_unquoted_all_elements_array_expansion(parameter: &ParameterExpansion) -> bool {
    let ParameterExpansionSyntax::Bourne(syntax) = &parameter.syntax else {
        return false;
    };

    match syntax {
        BourneParameterExpansion::Access { reference }
        | BourneParameterExpansion::Indices { reference }
        | BourneParameterExpansion::Slice { reference, .. } => {
            var_ref_uses_all_elements_at_splat(reference)
        }
        BourneParameterExpansion::Operation {
            reference,
            operator,
            ..
        } => {
            !matches!(operator, ParameterOp::UseReplacement)
                && var_ref_uses_all_elements_at_splat(reference)
        }
        BourneParameterExpansion::Transformation { reference, .. } => {
            var_ref_uses_all_elements_at_splat(reference)
        }
        _ => false,
    }
}

fn parameter_uses_direct_all_elements_array_expansion(parameter: &ParameterExpansion) -> bool {
    let ParameterExpansionSyntax::Bourne(syntax) = &parameter.syntax else {
        return false;
    };

    match syntax {
        BourneParameterExpansion::Access { reference }
        | BourneParameterExpansion::Indices { reference }
        | BourneParameterExpansion::Slice { reference, .. } => {
            var_ref_uses_all_elements_at_splat(reference)
        }
        BourneParameterExpansion::Operation {
            reference,
            operator,
            ..
        } => {
            !matches!(operator, ParameterOp::UseReplacement)
                && var_ref_uses_all_elements_at_splat(reference)
        }
        BourneParameterExpansion::Transformation { reference, .. } => {
            var_ref_uses_all_elements_at_splat(reference)
        }
        _ => false,
    }
}

fn parameter_uses_replacement_all_elements_array_expansion(parameter: &ParameterExpansion) -> bool {
    let ParameterExpansionSyntax::Bourne(syntax) = &parameter.syntax else {
        return false;
    };

    matches!(
        syntax,
        BourneParameterExpansion::Operation {
            reference,
            operator: ParameterOp::UseReplacement,
            ..
        } if var_ref_uses_all_elements_at_splat(reference)
    )
}

fn parameter_is_unindexed_bash_source(parameter: &ParameterExpansion) -> bool {
    let ParameterExpansionSyntax::Bourne(syntax) = &parameter.syntax else {
        return false;
    };

    matches!(
        syntax,
        BourneParameterExpansion::Access { reference }
            if reference.name.as_str() == "BASH_SOURCE" && reference.subscript.is_none()
    )
}

fn parameter_uses_star_splat(parameter: &ParameterExpansion) -> bool {
    let ParameterExpansionSyntax::Bourne(syntax) = &parameter.syntax else {
        return false;
    };

    match syntax {
        BourneParameterExpansion::Access { reference }
        | BourneParameterExpansion::Slice { reference, .. }
        | BourneParameterExpansion::Operation { reference, .. }
        | BourneParameterExpansion::Transformation { reference, .. } => {
            var_ref_uses_star_splat(reference)
        }
        _ => false,
    }
}

fn var_ref_uses_positional_at_splat(reference: &VarRef) -> bool {
    reference.name.as_str() == "@"
}

fn parameter_uses_positional_at_splat(parameter: &ParameterExpansion) -> bool {
    let ParameterExpansionSyntax::Bourne(syntax) = &parameter.syntax else {
        return false;
    };

    match syntax {
        BourneParameterExpansion::Access { reference }
        | BourneParameterExpansion::Slice { reference, .. }
        | BourneParameterExpansion::Operation { reference, .. } => {
            var_ref_uses_positional_at_splat(reference)
        }
        _ => false,
    }
}

fn parameter_is_pure_positional_at_splat(parameter: &ParameterExpansion) -> bool {
    let ParameterExpansionSyntax::Bourne(syntax) = &parameter.syntax else {
        return false;
    };

    match syntax {
        BourneParameterExpansion::Access { reference }
        | BourneParameterExpansion::Slice { reference, .. } => {
            var_ref_uses_positional_at_splat(reference)
        }
        _ => false,
    }
}

fn parameter_uses_assign_default_operator(parameter: &ParameterExpansion) -> bool {
    let ParameterExpansionSyntax::Bourne(syntax) = &parameter.syntax else {
        return false;
    };

    match syntax {
        BourneParameterExpansion::Operation { operator, .. } => {
            matches!(operator, ParameterOp::AssignDefault)
        }
        BourneParameterExpansion::Indirect {
            operator: Some(operator),
            ..
        } => matches!(operator, ParameterOp::AssignDefault),
        _ => false,
    }
}

fn collect_unquoted_star_splat_spans(parts: &[WordPartNode], quoted: bool, spans: &mut Vec<Span>) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_unquoted_star_splat_spans(parts, true, spans);
            }
            _ if !quoted && part_uses_star_splat(&part.kind) => spans.push(part.span),
            _ => {}
        }
    }
}

fn collect_quoted_star_splat_spans(parts: &[WordPartNode], quoted: bool, spans: &mut Vec<Span>) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_quoted_star_splat_spans(parts, true, spans);
            }
            _ if quoted && part_uses_star_splat(&part.kind) => spans.push(part.span),
            _ => {}
        }
    }
}

fn collect_unquoted_escaped_pipe_or_brace_spans(
    parts: &[WordPartNode],
    source: &str,
    quoted: bool,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_unquoted_escaped_pipe_or_brace_spans(parts, source, true, spans);
            }
            WordPart::Literal(_) if !quoted => {
                spans.extend(literal_escaped_pipe_or_brace_spans(part.span, source));
            }
            WordPart::Literal(_)
            | WordPart::Variable(_)
            | WordPart::CommandSubstitution { .. }
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
            | WordPart::ProcessSubstitution { .. }
            | WordPart::Transformation { .. }
            | WordPart::ZshQualifiedGlob(_) => {}
        }
    }
}

fn literal_escaped_pipe_or_brace_spans(span: Span, source: &str) -> Vec<Span> {
    let text = span.slice(source);
    let bytes = text.as_bytes();
    if bytes.len() < 2 {
        return Vec::new();
    }

    let mut spans = Vec::new();
    for index in 0..(bytes.len() - 1) {
        if bytes[index] != b'\\' || byte_is_backslash_escaped(bytes, index) {
            continue;
        }
        if !matches!(bytes[index + 1], b'|' | b'{' | b'}') {
            continue;
        }

        let start = span.start.advanced_by(&text[..index]);
        let end = span.start.advanced_by(&text[..index + 2]);
        spans.push(Span::from_positions(start, end));
    }

    spans
}

fn collect_unquoted_assign_default_spans(
    parts: &[WordPartNode],
    quoted: bool,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_unquoted_assign_default_spans(parts, true, spans);
            }
            _ if !quoted && part_uses_assign_default_operator(&part.kind) => spans.push(part.span),
            _ => {}
        }
    }
}

fn is_non_dollar_single_quoted(part: &WordPartNode) -> bool {
    matches!(part.kind, WordPart::SingleQuoted { dollar: false, .. })
}

fn single_quoted_fragment_inner_text<'a>(part: &WordPartNode, source: &'a str) -> Option<&'a str> {
    let WordPart::SingleQuoted { dollar: false, .. } = part.kind else {
        return None;
    };

    part.span
        .slice(source)
        .strip_prefix('\'')
        .and_then(|text| text.strip_suffix('\''))
}

fn literal_contains_unquoted_word_chars(text: &str) -> bool {
    !text.is_empty()
        && text.as_bytes().iter().all(u8::is_ascii_alphanumeric)
        && text.as_bytes().iter().any(u8::is_ascii_alphanumeric)
}

fn collect_nested_dynamic_double_quote_spans(
    parts: &[WordPartNode],
    inside_double_quotes: bool,
    spans: &mut Vec<Span>,
) {
    for (index, part) in parts.iter().enumerate() {
        let WordPart::DoubleQuoted { parts: inner, .. } = &part.kind else {
            continue;
        };

        if inside_double_quotes
            && double_quoted_parts_contain_dynamic_content(inner)
            && (neighbor_is_literal(parts.get(index.wrapping_sub(1)))
                || neighbor_is_literal(parts.get(index + 1)))
        {
            spans.push(part.span);
        }

        collect_nested_dynamic_double_quote_spans(inner, true, spans);
    }
}

fn double_quoted_parts_contain_dynamic_content(parts: &[WordPartNode]) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::Literal(_) | WordPart::SingleQuoted { .. } => false,
        WordPart::DoubleQuoted { parts, .. } => double_quoted_parts_contain_dynamic_content(parts),
        WordPart::Variable(_)
        | WordPart::Parameter(_)
        | WordPart::CommandSubstitution { .. }
        | WordPart::ArithmeticExpansion { .. }
        | WordPart::ParameterExpansion { .. }
        | WordPart::Length(_)
        | WordPart::ArrayAccess(_)
        | WordPart::ArrayLength(_)
        | WordPart::ArrayIndices(_)
        | WordPart::Substring { .. }
        | WordPart::ArraySlice { .. }
        | WordPart::IndirectExpansion { .. }
        | WordPart::PrefixMatch { .. }
        | WordPart::ProcessSubstitution { .. }
        | WordPart::Transformation { .. }
        | WordPart::ZshQualifiedGlob(_) => true,
    })
}

fn neighbor_is_literal(part: Option<&WordPartNode>) -> bool {
    matches!(part.map(|part| &part.kind), Some(WordPart::Literal(_)))
}

fn collect_positional_at_splat_spans(parts: &[WordPartNode], spans: &mut Vec<Span>) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => collect_positional_at_splat_spans(parts, spans),
            _ if part_uses_positional_at_splat(&part.kind) => spans.push(part.span),
            _ => {}
        }
    }
}

fn parts_are_pure_positional_at_splat(parts: &[WordPartNode]) -> bool {
    let mut saw_splat = false;

    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => return false,
            WordPart::DoubleQuoted { parts, .. } => {
                if !parts_are_pure_positional_at_splat(parts) {
                    return false;
                }
                saw_splat = true;
            }
            _ if part_is_pure_positional_at_splat(&part.kind) => saw_splat = true,
            _ => return false,
        }
    }

    saw_splat
}

fn word_has_single_positional_at_splat_part(word: &Word) -> bool {
    parts_have_single_positional_at_splat(&word.parts)
}

fn parts_have_single_positional_at_splat(parts: &[WordPartNode]) -> bool {
    let [part] = parts else {
        return false;
    };

    match &part.kind {
        WordPart::DoubleQuoted { parts, .. } => parts_have_single_positional_at_splat(parts),
        WordPart::SingleQuoted { .. } => false,
        _ => part_uses_positional_at_splat(&part.kind),
    }
}

fn word_has_single_folded_all_elements_array_part(word: &Word) -> bool {
    parts_have_single_folded_all_elements_array_part(&word.parts)
}

fn parts_have_single_folded_all_elements_array_part(parts: &[WordPartNode]) -> bool {
    let [part] = parts else {
        return false;
    };

    match &part.kind {
        WordPart::DoubleQuoted { parts, .. } => {
            parts_have_single_folded_all_elements_array_part(parts)
        }
        WordPart::SingleQuoted { .. } => false,
        WordPart::Parameter(parameter) => {
            part_uses_direct_all_elements_array_expansion(&part.kind)
                || parameter_uses_replacement_all_elements_array_expansion(parameter)
        }
        _ => part_uses_direct_all_elements_array_expansion(&part.kind),
    }
}

fn positional_at_splat_is_standalone_expansion(word: &Word, source: &str) -> bool {
    let text = word.span.slice(source);
    let body = if word.is_fully_double_quoted() {
        let Some(unquoted) = text
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
        else {
            return false;
        };
        unquoted
    } else {
        text
    };

    if body == "$@" || body == "${@}" {
        return true;
    }

    if !body.starts_with("${@") || !body.ends_with('}') {
        return false;
    }
    true
}

fn all_elements_array_expansion_is_standalone(word: &Word, source: &str) -> bool {
    if word.parts.len() != 1 {
        return false;
    }

    let text = word.span.slice(source);
    let body = if word.is_fully_double_quoted() {
        let Some(unquoted) = text
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
        else {
            return false;
        };
        unquoted
    } else {
        text
    };

    folded_all_elements_array_candidate_spans(word, source)
        .first()
        .is_some_and(|span| span.slice(source) == body)
}

fn span_is_backslash_escaped(span: Span, source: &str) -> bool {
    offset_is_backslash_escaped(span.start.offset, source)
}

fn offset_is_backslash_escaped(offset: usize, source: &str) -> bool {
    if offset == 0 {
        return false;
    }

    let bytes = source.as_bytes();
    let mut index = offset;
    let mut backslash_count = 0usize;
    while index > 0 && bytes[index - 1] == b'\\' {
        backslash_count += 1;
        index -= 1;
    }

    backslash_count % 2 == 1
}

fn span_is_escaped(span: Span, source: &str) -> bool {
    span_is_backslash_escaped(span, source)
}

#[cfg(test)]
mod tests {
    use shuck_ast::Span;
    use shuck_parser::parser::Parser;

    use super::{
        all_elements_array_expansion_part_spans, array_expansion_part_spans,
        backtick_double_escaped_parameter_spans, backtick_escaped_parameters,
        backtick_substitution_spans, command_substitution_part_spans, find_extglob_bounds,
        line_has_escaped_newline_continuation, position_at_offset, scalar_expansion_part_spans,
        shellcheck_collapsed_backtick_part_span_in_source,
        unquoted_all_elements_array_expansion_part_spans,
        unquoted_command_substitution_part_spans_in_source,
        unquoted_dollar_paren_command_substitution_part_spans_in_source,
        unquoted_scalar_expansion_part_spans, word_all_elements_array_slice_span_in_source,
        word_all_elements_array_slice_spans, word_caret_negated_bracket_spans,
        word_double_quoted_scalar_only_expansion_spans, word_exactly_one_extglob_span,
        word_folded_all_elements_array_span_in_source, word_folded_positional_at_splat_span,
        word_folded_positional_at_splat_span_in_source,
        word_has_direct_all_elements_array_expansion_in_source,
        word_has_folded_positional_at_splat, word_has_quoted_all_elements_array_slice,
        word_has_unquoted_brace_expansion, word_is_pure_positional_at_splat,
        word_nested_dynamic_double_quote_spans, word_positional_at_splat_span_in_source,
        word_positional_at_splat_spans, word_quoted_all_elements_array_slice_spans,
        word_quoted_star_splat_spans, word_quoted_unindexed_bash_source_span_in_source,
        word_starts_with_extglob, word_suspicious_bracket_glob_spans,
        word_unquoted_assign_default_spans, word_unquoted_escaped_pipe_or_brace_spans_in_source,
        word_unquoted_glob_pattern_spans, word_unquoted_glob_pattern_spans_outside_brace_expansion,
        word_unquoted_scalar_between_double_quoted_segments_spans, word_unquoted_star_splat_spans,
        word_unquoted_word_after_single_quoted_segment_spans,
    };

    #[test]
    fn command_substitution_spans_use_inner_part_ranges() {
        let source = "printf '%s\\n' prefix$(date)suffix\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command_substitution_part_spans(&command.args[1]);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].slice(source), "$(date)");
    }

    #[test]
    fn array_expansion_spans_only_return_array_like_parts() {
        let source = "printf '%s\\n' ${arr[@]} ${arr[@]+fallback} ${arr[*]:-fallback} ${arr[*]@Q} ${arr[0]}\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .skip(1)
            .flat_map(|word| array_expansion_part_spans(word, source))
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();
        assert_eq!(
            spans,
            vec!["${arr[@]}", "${arr[*]:-fallback}", "${arr[*]@Q}"]
        );
    }

    #[test]
    fn scalar_expansion_spans_ignore_array_splats_and_command_substitutions() {
        let source = "printf '%s\\n' prefix${name}suffix ${arr[@]} ${arr[0]} ${arr[@]:-fallback} ${arr[*]:-fallback} ${arr[@]@Q} ${arr[*]@Q} ${arr[0]:-fallback} $(date)\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert_eq!(
            scalar_expansion_part_spans(&command.args[1], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${name}"]
        );
        assert!(
            scalar_expansion_part_spans(&command.args[2], source).is_empty(),
            "array splats should be left to S008"
        );
        assert_eq!(
            scalar_expansion_part_spans(&command.args[3], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[0]}"]
        );
        assert!(
            scalar_expansion_part_spans(&command.args[4], source).is_empty(),
            "array splats with default operators should be left to array rules"
        );
        assert!(
            scalar_expansion_part_spans(&command.args[5], source).is_empty(),
            "star-selector array splats with default operators should be left to array rules"
        );
        assert!(
            scalar_expansion_part_spans(&command.args[6], source).is_empty(),
            "array splat transformations should be left to array rules"
        );
        assert!(
            scalar_expansion_part_spans(&command.args[7], source).is_empty(),
            "star-splat transformations should stay on the star-parameter path"
        );
        assert_eq!(
            scalar_expansion_part_spans(&command.args[8], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[0]:-fallback}"]
        );
        assert!(
            scalar_expansion_part_spans(&command.args[9], source).is_empty(),
            "command substitutions should be left to S004"
        );
    }

    #[test]
    fn selector_helpers_distinguish_splats_from_indexed_and_quoted_keys() {
        let source = "printf '%s\\n' ${arr[@]} ${arr[*]} ${arr[0]} ${assoc[\"key\"]}\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert_eq!(command.args.len(), 5);
        assert_eq!(
            array_expansion_part_spans(&command.args[1], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[@]}"]
        );
        assert_eq!(
            array_expansion_part_spans(&command.args[2], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[*]}"]
        );
        assert!(array_expansion_part_spans(&command.args[3], source).is_empty());
        assert!(array_expansion_part_spans(&command.args[4], source).is_empty());

        assert!(scalar_expansion_part_spans(&command.args[1], source).is_empty());
        assert!(scalar_expansion_part_spans(&command.args[2], source).is_empty());
        assert_eq!(
            scalar_expansion_part_spans(&command.args[3], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[0]}"]
        );
        assert_eq!(
            scalar_expansion_part_spans(&command.args[4], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${assoc[\"key\"]}"]
        );
    }

    #[test]
    fn word_exactly_one_extglob_span_tracks_mixed_parts() {
        let source = "echo @($choice|bar)\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let span = word_exactly_one_extglob_span(&command.args[0], source)
            .expect("expected mixed-part extglob span");
        assert_eq!(span.slice(source), "@($choice|bar)");
    }

    #[test]
    fn find_extglob_bounds_detects_parenthesized_alternation() {
        assert_eq!(find_extglob_bounds(b"(foo|bar)*"), Some((0, 8)));
    }

    #[test]
    fn word_exactly_one_extglob_span_ignores_nested_command_source_text() {
        let source = "echo $(printf '@(foo|bar)')\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert!(word_exactly_one_extglob_span(&command.args[0], source).is_none());
    }

    #[test]
    fn word_starts_with_extglob_distinguishes_leading_and_trailing_groups() {
        let source = "printf '%s\\n' ?(*.txt) *.@(jpg|png)\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert!(word_starts_with_extglob(&command.args[1], source));
        assert!(!word_starts_with_extglob(&command.args[2], source));
    }

    #[test]
    fn word_caret_negated_bracket_spans_track_mixed_parts() {
        let source = "echo [^$chars]*\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = word_caret_negated_bracket_spans(&command.args[0], source);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].slice(source), "[^$chars]");
    }

    #[test]
    fn word_caret_negated_bracket_spans_ignore_nested_command_source_text() {
        let source = "echo $(printf '[^a]*')\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert!(word_caret_negated_bracket_spans(&command.args[0], source).is_empty());
    }

    #[test]
    fn word_unquoted_glob_pattern_spans_track_unquoted_segments_only() {
        let source = "echo foo*.txt \"bar?\" [ab] \"${name}\"*.tmp \\*.bak foo\\?bar \\[ab\\]\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert_eq!(
            word_unquoted_glob_pattern_spans(&command.args[0], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["*"]
        );
        assert!(
            word_unquoted_glob_pattern_spans(&command.args[1], source).is_empty(),
            "double-quoted wildcard should not be reported"
        );
        assert_eq!(
            word_unquoted_glob_pattern_spans(&command.args[2], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["[ab]"]
        );
        assert_eq!(
            word_unquoted_glob_pattern_spans(&command.args[3], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["*"]
        );
        assert!(
            word_unquoted_glob_pattern_spans(&command.args[4], source).is_empty(),
            "escaped wildcard should not be reported"
        );
        assert!(
            word_unquoted_glob_pattern_spans(&command.args[5], source).is_empty(),
            "escaped question mark should not be reported"
        );
        assert!(
            word_unquoted_glob_pattern_spans(&command.args[6], source).is_empty(),
            "escaped bracket expression should not be reported"
        );
    }

    #[test]
    fn word_unquoted_glob_pattern_spans_join_adjacent_literal_parts() {
        let source = "echo [/$]\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert_eq!(
            word_unquoted_glob_pattern_spans(&command.args[0], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["[/$]"]
        );
    }

    #[test]
    fn word_unquoted_glob_pattern_spans_ignore_parameter_operator_tails() {
        let source = r#"echo ${path/*\/} ${name#*:} ${name##*foo}"#;
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert!(
            word_unquoted_glob_pattern_spans(&command.args[0], source).is_empty(),
            "parameter replacement operator tails should not be reported as pathname globs"
        );
        assert!(
            word_unquoted_glob_pattern_spans(&command.args[1], source).is_empty(),
            "parameter prefix operator tails should not be reported as pathname globs"
        );
        assert!(
            word_unquoted_glob_pattern_spans(&command.args[2], source).is_empty(),
            "parameter longest-prefix operator tails should not be reported as pathname globs"
        );
    }

    #[test]
    fn word_unquoted_glob_pattern_spans_outside_brace_expansion_ignore_brace_local_globs() {
        let source = "\
echo $DIR/{1..3}*.txt \
$DIR/setjmp-aarch64/{setjmp.S,private-*.h} \
$PKG/usr/man/{ja/,}*/*-8.?.?.gz
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert_eq!(
            word_unquoted_glob_pattern_spans_outside_brace_expansion(&command.args[0], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["*"]
        );
        assert!(
            word_unquoted_glob_pattern_spans_outside_brace_expansion(&command.args[1], source)
                .is_empty(),
            "globs nested inside brace alternatives should stay excluded"
        );
        assert_eq!(
            word_unquoted_glob_pattern_spans_outside_brace_expansion(&command.args[2], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["*", "*", "?", "?"]
        );
    }

    #[test]
    fn word_suspicious_bracket_glob_spans_track_duplicate_literal_members() {
        let source = "\
echo [appname] [1,2,3] [foo-bar] foo[aba]bar [start\\|stop\\|restart] \"$dir\"/[appname]
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(|word| word_suspicious_bracket_glob_spans(word, source))
            .map(|span: Span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(
            spans,
            vec![
                "[appname]",
                "[1,2,3]",
                "[foo-bar]",
                "[aba]",
                "[start\\|stop\\|restart]",
                "[appname]"
            ]
        );
    }

    #[test]
    fn word_suspicious_bracket_glob_spans_treat_utf8_members_as_characters() {
        let source = "echo [ÅÄ] [ÅÅ] [éç] [éé]\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(|word| word_suspicious_bracket_glob_spans(word, source))
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["[ÅÅ]", "[éé]"]);
    }

    #[test]
    fn word_suspicious_bracket_glob_spans_ignore_valid_sets_and_named_classes() {
        let source = "\
echo [ab] [a-z] [123] [1,2] [bar] [[:alpha:]] [![:digit:]] [:lower:] [a-zA-Z_] [0-9a-fA-F] foo[xyz]bar \\[appname\\]
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(|word| word_suspicious_bracket_glob_spans(word, source))
            .collect::<Vec<_>>();

        assert!(spans.is_empty(), "spans: {spans:?}");
    }

    #[test]
    fn word_has_unquoted_brace_expansion_detects_sequence_forms() {
        let source = "echo {foo,bar} {1..3} ${dir}/{a..c}*.txt\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert!(word_has_unquoted_brace_expansion(&command.args[0], source));
        assert!(word_has_unquoted_brace_expansion(&command.args[1], source));
        assert!(word_has_unquoted_brace_expansion(&command.args[2], source));
    }

    #[test]
    fn all_elements_array_expansion_spans_only_return_at_style_parts() {
        let source =
            "printf '%s\\n' $@ $* \"$@\" \"$*\" ${arr[@]} ${arr[*]} ${arr[@]:1:2} ${arr[*]:1:2}\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert_eq!(
            all_elements_array_expansion_part_spans(&command.args[1], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$@"]
        );
        assert!(all_elements_array_expansion_part_spans(&command.args[2], source).is_empty());
        assert_eq!(
            all_elements_array_expansion_part_spans(&command.args[3], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$@"]
        );
        assert!(all_elements_array_expansion_part_spans(&command.args[4], source).is_empty());
        assert_eq!(
            all_elements_array_expansion_part_spans(&command.args[5], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[@]}"]
        );
        assert!(all_elements_array_expansion_part_spans(&command.args[6], source).is_empty());
        assert_eq!(
            all_elements_array_expansion_part_spans(&command.args[7], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[@]:1:2}"]
        );
        assert!(all_elements_array_expansion_part_spans(&command.args[8], source).is_empty());
    }

    #[test]
    fn all_elements_array_expansion_spans_normalize_parser_misalignment() {
        let source = "\
#!/bin/bash
shims=(a)
eval \\
\"conda_shim() {
  case \\\"\\${1##*/}\\\" in
    ${shims[@]}
    *) return 1;;
  esac
}\"
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[1].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = all_elements_array_expansion_part_spans(&command.args[0], source);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].slice(source), "${shims[@]}");
        assert_eq!(spans[0].start.column, 5);
        assert_eq!(spans[0].end.column, 16);
    }

    #[test]
    fn all_elements_array_expansion_spans_ignore_escaped_literal_expansions() {
        let source = "\
#!/bin/bash
eval command sudo \\\"\\${sudo_args[@]}\\\" \\\"\\$@\\\"
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert!(all_elements_array_expansion_part_spans(&command.args[2], source).is_empty());
    }

    #[test]
    fn escaped_newline_continuations_require_an_odd_backslash_count() {
        assert!(line_has_escaped_newline_continuation("echo foo \\"));
        assert!(line_has_escaped_newline_continuation("echo foo \\\\\\"));
        assert!(!line_has_escaped_newline_continuation("echo foo \\\\"));
        assert!(!line_has_escaped_newline_continuation("echo foo"));
        assert!(!line_has_escaped_newline_continuation("echo foo \\   "));
        assert!(!line_has_escaped_newline_continuation("echo foo \\\\   "));
        assert!(line_has_escaped_newline_continuation("echo foo \\\r"));
        assert!(!line_has_escaped_newline_continuation("echo foo \\\\\r"));
        assert!(!line_has_escaped_newline_continuation(r"printf 'foo\"));
        assert!(!line_has_escaped_newline_continuation(r"printf # foo\"));
        assert!(line_has_escaped_newline_continuation(r#"printf "foo\"#));
    }

    #[test]
    fn all_elements_array_expansion_spans_track_safe_quoted_name_fanout() {
        let source = "\
printf '%s\\n' ${#arr[@]} ${!arr[@]} ${!cfg@} ${name:-safe[@]} ${arr[@]}
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert!(all_elements_array_expansion_part_spans(&command.args[1], source).is_empty());
        assert_eq!(
            all_elements_array_expansion_part_spans(&command.args[2], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${!arr[@]}"]
        );
        assert_eq!(
            all_elements_array_expansion_part_spans(&command.args[3], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${!cfg@}"]
        );
        assert!(all_elements_array_expansion_part_spans(&command.args[4], source).is_empty());
        assert_eq!(
            all_elements_array_expansion_part_spans(&command.args[5], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[@]}"]
        );
    }

    #[test]
    fn unquoted_all_elements_array_expansion_spans_only_return_unquoted_at_style_parts() {
        let source = "printf '%s\\n' $@ $* \"$@\" \"$*\" ${arr[@]} ${arr[*]} ${arr[@]:1:2} ${arr[*]:1:2} ${!arr[@]} ${arr[@]/#/#} ${arr[@]@Q} ${arr[@]:-fallback} ${arr[@]:+fallback} ${1+\"$@\"}\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert_eq!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[1], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$@"]
        );
        assert!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[2], source).is_empty()
        );
        assert!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[3], source).is_empty()
        );
        assert!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[4], source).is_empty()
        );
        assert_eq!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[5], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[@]}"]
        );
        assert!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[6], source).is_empty()
        );
        assert_eq!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[7], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[@]:1:2}"]
        );
        assert!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[8], source).is_empty()
        );
        assert_eq!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[9], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${!arr[@]}"]
        );
        assert_eq!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[10], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[@]/#/#}"]
        );
        assert_eq!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[11], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[@]@Q}"]
        );
        assert_eq!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[12], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[@]:-fallback}"]
        );
        assert!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[13], source).is_empty()
        );
        assert!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[14], source).is_empty()
        );
    }

    #[test]
    fn positional_parameters_are_treated_like_array_splats() {
        let source = "printf '%s\\n' $@ $* \"$@\" \"$*\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert_eq!(
            array_expansion_part_spans(&command.args[1], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$@"]
        );
        assert_eq!(
            array_expansion_part_spans(&command.args[2], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$*"]
        );
    }

    #[test]
    fn word_all_elements_array_slice_spans_track_at_selector_slice_forms_only() {
        let source = "\
printf '%s\\n' ${@:2} ${@:2:3} ${arr[@]:1} ${arr[@]:1:2} ${arr[*]:1} ${*:2} ${arr[0]:1} ${@} ${arr[@]}
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(word_all_elements_array_slice_spans)
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(
            spans,
            vec!["${@:2}", "${@:2:3}", "${arr[@]:1}", "${arr[@]:1:2}"]
        );
    }

    #[test]
    fn word_quoted_all_elements_array_slice_spans_track_only_quoted_forms() {
        let source = "\
printf '%s\\n' \"${@:2}\" \"x${@:2}y\" \"${arr[@]:1}\" \"${arr[@]:1:2}\" ${@:2} \"${arr[*]:1}\" \"${*:2}\" \"\\${@:2}\" \"${@:-fallback}\" \"${@}\" \"${arr[@]}\"
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(word_quoted_all_elements_array_slice_spans)
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(
            spans,
            vec!["${@:2}", "${@:2}", "${arr[@]:1}", "${arr[@]:1:2}"]
        );
        assert!(word_has_quoted_all_elements_array_slice(&command.args[1]));
        assert!(!word_has_quoted_all_elements_array_slice(&command.args[5]));
    }

    #[test]
    fn word_has_direct_all_elements_array_expansion_ignores_nested_or_scalar_operator_uses() {
        let source = "\
printf '%s\\n' \"$@\" \"${arr[@]}\" \"${arr[@]:1}\" \"${arr[@]:-fallback}\" \"${@:+ok}\" \"${arr[@]:+ok}\" \"${target=\"$@\"}\" \"$(echo \"$@\")\" \"${arr[*]}\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let matches = command
            .args
            .iter()
            .skip(1)
            .map(|word| word_has_direct_all_elements_array_expansion_in_source(word, source))
            .collect::<Vec<_>>();

        assert_eq!(
            matches,
            vec![true, true, true, true, false, false, false, false, false]
        );
    }

    #[test]
    fn word_has_direct_all_elements_array_expansion_handles_backslash_parity() {
        let source = "\
printf '%s\\n' \"\\$@\" \"\\\\$@\" \"\\${@:2}\" \"\\\\${@:2}\" \"\\${arr[@]}\" \"\\\\${arr[@]}\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let matches = command
            .args
            .iter()
            .skip(1)
            .map(|word| word_has_direct_all_elements_array_expansion_in_source(word, source))
            .collect::<Vec<_>>();

        assert_eq!(matches, vec![false, true, false, true, false, true]);
    }

    #[test]
    fn word_has_direct_all_elements_array_expansion_ignores_escaped_parameter_nesting() {
        let source = "\
printf '%s\\n' \"\\${1+'\\\"$@\\\"'}\" \"$@\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let matches = command
            .args
            .iter()
            .skip(1)
            .map(|word| word_has_direct_all_elements_array_expansion_in_source(word, source))
            .collect::<Vec<_>>();

        assert_eq!(matches, vec![false, true]);
    }

    #[test]
    fn word_has_direct_all_elements_array_expansion_ignores_quoted_braces_in_escaped_text() {
        let source = "\
printf '%s\\n' \"\\${1+'} \\\"$@\\\"'}\" \"$@\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let matches = command
            .args
            .iter()
            .skip(1)
            .map(|word| word_has_direct_all_elements_array_expansion_in_source(word, source))
            .collect::<Vec<_>>();

        assert_eq!(matches, vec![false, true]);
    }

    #[test]
    fn word_all_elements_array_slice_span_in_source_ignores_escaped_markers() {
        let source = "printf '%s\\n' \"\\${arr[@]:1}\" \"${arr[@]:1}\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert!(word_all_elements_array_slice_span_in_source(&command.args[1], source).is_none());
        assert_eq!(
            word_all_elements_array_slice_span_in_source(&command.args[2], source)
                .expect("expected array slice span")
                .slice(source),
            "${arr[@]:1}"
        );
    }

    #[test]
    fn word_quoted_unindexed_bash_source_span_in_source_tracks_scalar_forms() {
        let source = "\
printf '%s\\n' \"$BASH_SOURCE\" \"${BASH_SOURCE}\" \"$(dirname \"$BASH_SOURCE\")\" \"${BASH_SOURCE[0]}\" \"\\$BASH_SOURCE\"
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert_eq!(
            word_quoted_unindexed_bash_source_span_in_source(&command.args[1], source)
                .expect("expected BASH_SOURCE span")
                .slice(source),
            "$BASH_SOURCE"
        );
        assert_eq!(
            word_quoted_unindexed_bash_source_span_in_source(&command.args[2], source)
                .expect("expected BASH_SOURCE span")
                .slice(source),
            "${BASH_SOURCE}"
        );
        assert!(
            word_quoted_unindexed_bash_source_span_in_source(&command.args[3], source).is_none()
        );
        assert!(
            word_quoted_unindexed_bash_source_span_in_source(&command.args[4], source).is_none()
        );
        assert!(
            word_quoted_unindexed_bash_source_span_in_source(&command.args[5], source).is_none()
        );
    }

    #[test]
    fn word_unquoted_star_splat_spans_tracks_star_selector_forms_only() {
        let source = "\
printf '%s\\n' $* ${*} ${*:1} ${arr[*]} ${arr[*]:1:2} ${arr[*]:-fallback} ${arr[*]@Q} ${!arr[*]} ${arr[@]} ${arr[@]:1} ${arr[0]} \"$*\" \"${arr[*]}\"
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(word_unquoted_star_splat_spans)
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(
            spans,
            vec![
                "$*",
                "${*}",
                "${*:1}",
                "${arr[*]}",
                "${arr[*]:1:2}",
                "${arr[*]:-fallback}",
                "${arr[*]@Q}"
            ]
        );
    }

    #[test]
    fn word_unquoted_star_parameter_spans_tracks_star_selector_forms_only() {
        let source = "\
printf '%s\\n' $* ${arr[*]} ${arr[*]:1:2} ${arr[*]:-fallback} ${arr[*]@Q} ${!arr[*]} ${arr[@]} ${arr[@]:1} ${arr[0]} \"$*\" \"${arr[*]}\"
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(|word| {
                let unquoted_array_spans = super::unquoted_array_expansion_part_spans(word, source);
                super::word_unquoted_star_parameter_spans(word, &unquoted_array_spans)
            })
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(
            spans,
            vec![
                "$*",
                "${arr[*]}",
                "${arr[*]:1:2}",
                "${arr[*]:-fallback}",
                "${arr[*]@Q}"
            ]
        );
    }

    #[test]
    fn word_quoted_star_splat_spans_tracks_double_quoted_star_selector_forms_only() {
        let source = "\
printf '%s\\n' \"$*\" \"${*}\" \"${*:1}\" \"${arr[*]}\" \"${arr[*]:1:2}\" \"${!arr[*]}\" \"${arr[@]}\" \"${arr[@]:1}\" \"$@\" ${arr[*]}
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(word_quoted_star_splat_spans)
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(
            spans,
            vec!["$*", "${*}", "${*:1}", "${arr[*]}", "${arr[*]:1:2}"]
        );
    }

    #[test]
    fn word_unquoted_assign_default_spans_track_only_unquoted_assignment_defaults() {
        let source = "\
printf '%s\\n' ${x=} ${x:=a} ${x:-a} \"${x=}\" \"${x:=a}\" prefix${x=}suffix ${!name:=fallback} ${name/pat/repl}
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(word_unquoted_assign_default_spans)
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(
            spans,
            vec!["${x=}", "${x:=a}", "${x=}", "${!name:=fallback}"]
        );
    }

    #[test]
    fn word_unquoted_escaped_pipe_or_brace_spans_track_only_unquoted_literal_sequences() {
        let source = "\
printf '%s\\n' mode\\|verbose token\\{a,b\\} token\\}end \"mode\\|verbose\" 'token\\{a,b\\}'
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(|word| word_unquoted_escaped_pipe_or_brace_spans_in_source(word, source))
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["\\|", "\\{", "\\}", "\\}"]);
    }

    #[test]
    fn word_unquoted_word_after_single_quoted_segment_spans_tracks_literal_suffix_words() {
        let source = "\
printf '%s\\n' 'foo'Default'baz' 'foo'123'baz' 'foo'-'baz' 'foo''baz' 'foo'$bar'baz' $'foo'Default'baz' '/x/'d ^default'\\s'via 'left'lib$SUFFIX'right' 'left'fuzz_ng_$SUFFIX'right'
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(|word| word_unquoted_word_after_single_quoted_segment_spans(word, source))
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["Default", "123", "d", "via", "lib"]);
    }

    #[test]
    fn word_unquoted_word_after_single_quoted_segment_ignores_escaped_quote_bridges() {
        let source = "\
printf '%s\\n' 's/foo/'\\''bar'\\''/g' 'foo'Default'baz'
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(|word| word_unquoted_word_after_single_quoted_segment_spans(word, source))
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["Default"]);
    }

    #[test]
    fn word_unquoted_scalar_between_double_quoted_segments_tracks_dynamic_middle_parts() {
        let source = "\
printf '%s\\n' \"$a\"$b\"$c\" \"left \"$d\"\" \"\"$e\" right\" \"left \"$(printf '%s' ok)\" right\" \"a\"b\"c\" prefix\"$f\"suffix \"$g\"$@\"$h\"
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(|word| {
                let unquoted_scalar_spans = unquoted_scalar_expansion_part_spans(word, source)
                    .into_iter()
                    .chain(unquoted_command_substitution_part_spans_in_source(
                        word, source,
                    ))
                    .collect::<Vec<_>>();
                word_unquoted_scalar_between_double_quoted_segments_spans(
                    word,
                    &unquoted_scalar_spans,
                )
            })
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["$b", "$d", "$e", "$(printf '%s' ok)"]);
    }

    #[test]
    fn unquoted_dollar_paren_command_substitution_spans_skip_legacy_backticks() {
        let source = "\
printf '%s\\n' \"left \"$(printf '%s' dollar)\" right\" \"left \"`printf '%s' tick`\" right\"
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(|word| {
                unquoted_dollar_paren_command_substitution_part_spans_in_source(word, source)
            })
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["$(printf '%s' dollar)"]);
    }

    #[test]
    fn word_double_quoted_scalar_only_expansion_spans_ignore_literal_affixes() {
        let source = "\
printf '%s\\n' \"$a\" \"$a\"\"$b\" \"prefix$a\" \"$a$(printf '%s' x)\" $a \"$a\"/\"$b\"
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(word_double_quoted_scalar_only_expansion_spans)
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["$a", "$a", "$b"]);
    }

    #[test]
    fn word_nested_dynamic_double_quote_spans_track_reopened_quotes_inside_outer_quotes() {
        let source = "\
printf '%s\\n' \"\n-DLZ4_HOME=\"${TERMUX_PREFIX}\"\n-DPROTOBUF_HOME=\"$(printf '%s' proto)\"\n\"\n
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = word_nested_dynamic_double_quote_spans(&command.args[1])
            .into_iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, Vec::<&str>::new());
    }

    #[test]
    fn word_positional_at_splat_spans_tracks_positional_forms_only() {
        let source = "\
printf '%s\\n' $@ ${@} ${@:1:2} \"${@}\" \"x$@y\" ${array[@]} ${array[@]:1} $* \"${*}\" ${!@}
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(word_positional_at_splat_spans)
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["$@", "${@}", "${@:1:2}", "${@}", "$@"]);
    }

    #[test]
    fn word_is_pure_positional_at_splat_rejects_mixed_words() {
        let source = "\
printf '%s\\n' \"$@\" ${@} \"${@:1}\" \"$@$@\" \"prefix$@suffix\" ${array[@]} \"$*\" \"$1\" \"${@:-fallback}\"
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let pure = command
            .args
            .iter()
            .map(word_is_pure_positional_at_splat)
            .collect::<Vec<_>>();

        assert_eq!(
            pure,
            vec![
                false, true, true, true, true, false, false, false, false, false
            ]
        );
    }

    #[test]
    fn word_folded_positional_at_splat_span_tracks_only_folding_forms() {
        let source = "\
printf '%s\\n' \"$@\" \"${@}\" \"${@:1}\" \"$@$@\" \"$@\"\"$@\" \"x$@y\" x$@y ${@} ${@:1} ${@:-fallback}
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let folded = command
            .args
            .iter()
            .filter_map(word_folded_positional_at_splat_span)
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(folded, vec!["$@", "$@", "$@", "$@"]);
        assert!(!word_has_folded_positional_at_splat(&command.args[1]));
        assert!(word_has_folded_positional_at_splat(&command.args[4]));
    }

    #[test]
    fn word_folded_positional_at_splat_span_in_source_ignores_standalone_expansions() {
        let source = "\
exec \"$@\" \"${@}\" \"${@:1}\" \"${@:-fallback}\" \"${@:${args_offset}}\" \"${@//-I\\/usr\\/include/-I${XBPS_CROSS_BASE}\\/usr\\/include}\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert!(command.args.iter().all(|word| {
            word_folded_positional_at_splat_span_in_source(word, source).is_none()
        }));
    }

    #[test]
    fn word_folded_positional_at_splat_span_in_source_ignores_escaped_positional_markers() {
        let source = "eval command \"\\$@\" \"x\\$@y\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert!(word_folded_positional_at_splat_span_in_source(&command.args[0], source).is_none());
        assert!(word_folded_positional_at_splat_span_in_source(&command.args[1], source).is_none());
    }

    #[test]
    fn word_folded_positional_at_splat_span_in_source_tracks_unescaped_splats_after_escaped_literals()
     {
        let source = "echo \"gvm_pkgset_use: \\$@   => $@\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert_eq!(
            word_folded_positional_at_splat_span_in_source(&command.args[0], source)
                .expect("expected folded positional span")
                .slice(source),
            "$@"
        );
    }

    #[test]
    fn word_folded_all_elements_array_span_in_source_tracks_array_splats_in_larger_words() {
        let source = "\
printf '%s\\n' \"${arr[@]}\" \"x${arr[@]}\" \"x${!arr[@]}\" \"x${arr[@]:1}\" \"x${arr[@]/a/b}\" \"x${arr[*]}\" \"\\${arr[@]}\" \"$@\" \"x$@\" \"${arr[@]+ ${arr[*]}}\" \"x${arr[@]+ ${arr[*]}}\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let folded = command
            .args
            .iter()
            .filter_map(|word| word_folded_all_elements_array_span_in_source(word, source))
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(
            folded,
            vec![
                "${arr[@]}",
                "${!arr[@]}",
                "${arr[@]:1}",
                "${arr[@]/a/b}",
                "$@",
                "${arr[@]+ ${arr[*]}}"
            ]
        );
    }

    #[test]
    fn shellcheck_collapsed_backtick_part_span_in_source_ignores_single_quoted_backticks() {
        let source = "printf '%s\\n' '`'\\\n  \"$foo\"\\\n  '`'\n";
        let start_offset = source.find("$foo").expect("expected expansion");
        let end_offset = start_offset + "$foo".len();
        let span = Span::from_positions(
            position_at_offset(source, start_offset).expect("expected start position"),
            position_at_offset(source, end_offset).expect("expected end position"),
        );

        assert_eq!(
            shellcheck_collapsed_backtick_part_span_in_source(span, source),
            span
        );
    }

    #[test]
    fn shellcheck_collapsed_backtick_part_span_in_source_ignores_backticks_in_comments() {
        let source = "# `\nprintf '%s\\n' \\\n  \"$foo\"\n# `\n";
        let start_offset = source.find("$foo").expect("expected expansion");
        let end_offset = start_offset + "$foo".len();
        let span = Span::from_positions(
            position_at_offset(source, start_offset).expect("expected start position"),
            position_at_offset(source, end_offset).expect("expected end position"),
        );

        assert_eq!(
            shellcheck_collapsed_backtick_part_span_in_source(span, source),
            span
        );
    }

    #[test]
    fn shellcheck_collapsed_backtick_part_span_in_source_ignores_single_quoted_backslashes() {
        let source = "echo `printf '%s\\n' 'foo\\\n$bar'`\n";
        let start_offset = source.find("$bar").expect("expected expansion");
        let end_offset = start_offset + "$bar".len();
        let span = Span::from_positions(
            position_at_offset(source, start_offset).expect("expected start position"),
            position_at_offset(source, end_offset).expect("expected end position"),
        );

        assert_eq!(
            shellcheck_collapsed_backtick_part_span_in_source(span, source),
            span
        );
    }

    #[test]
    fn shellcheck_collapsed_backtick_part_span_in_source_preserves_multiline_single_quote_context()
    {
        let source = "echo `printf '%s\\n' '\nfoo\\\n$bar'`\n";
        let start_offset = source.find("$bar").expect("expected expansion");
        let end_offset = start_offset + "$bar".len();
        let span = Span::from_positions(
            position_at_offset(source, start_offset).expect("expected start position"),
            position_at_offset(source, end_offset).expect("expected end position"),
        );

        assert_eq!(
            shellcheck_collapsed_backtick_part_span_in_source(span, source),
            span
        );
    }

    #[test]
    fn shellcheck_collapsed_backtick_part_span_in_source_clears_escape_state_after_continuations() {
        let source = "echo `printf '%s\\n' foo\\\n'$bar\\\n'\n$baz`\n";
        let start_offset = source.find("$baz").expect("expected expansion");
        let end_offset = start_offset + "$baz".len();
        let span = Span::from_positions(
            position_at_offset(source, start_offset).expect("expected start position"),
            position_at_offset(source, end_offset).expect("expected end position"),
        );

        assert_eq!(
            shellcheck_collapsed_backtick_part_span_in_source(span, source),
            span
        );
    }

    #[test]
    fn shellcheck_collapsed_backtick_part_span_in_source_counts_removed_backslash_pairs() {
        let source = r#"echo `sed -e "s/'/'\\\\\''/g" $2`"#;
        let span = span_for_text(source, "$2");

        let adjusted = shellcheck_collapsed_backtick_part_span_in_source(span, source);

        assert_eq!(adjusted.start.line, span.start.line);
        assert_eq!(adjusted.end.line, span.end.line);
        assert_eq!(adjusted.start.column, span.start.column - 2);
        assert_eq!(adjusted.end.column, span.end.column - 2);
    }

    #[test]
    fn shellcheck_collapsed_backtick_part_span_in_source_counts_escaped_dollars() {
        let source = r#"echo `echo \$x $y`"#;
        let span = span_for_text(source, "$y");

        let adjusted = shellcheck_collapsed_backtick_part_span_in_source(span, source);

        assert_eq!(adjusted.start.column, span.start.column - 1);
        assert_eq!(adjusted.end.column, span.end.column - 1);
    }

    #[test]
    fn shellcheck_collapsed_backtick_part_span_in_source_keeps_literal_backslashes() {
        let source = r#"echo `echo \a $x`"#;
        let span = span_for_text(source, "$x");

        assert_eq!(
            shellcheck_collapsed_backtick_part_span_in_source(span, source),
            span
        );
    }

    #[test]
    fn backtick_escaped_parameters_keep_quoted_assignment_prefixes_together() {
        let source = "`VAR=\"a b\" OTHER=$(printf '%s\\n' value) \\$cmd arg`";
        let backtick_spans = backtick_substitution_spans(source);
        let escaped = backtick_escaped_parameters(source, &backtick_spans);

        assert_eq!(escaped.len(), 1);
        assert!(escaped[0].standalone_command_name);
    }

    #[test]
    fn backtick_escaped_parameters_accept_append_assignment_prefixes() {
        let source = "`VAR+=x \\$cmd arg`";
        let backtick_spans = backtick_substitution_spans(source);
        let escaped = backtick_escaped_parameters(source, &backtick_spans);

        assert_eq!(escaped.len(), 1);
        assert!(escaped[0].standalone_command_name);
    }

    #[test]
    fn backtick_escaped_parameters_accept_redirection_prefixes() {
        let source = "`>/tmp/out 2>\"/tmp err\" FOO=bar \\$cmd arg`";
        let backtick_spans = backtick_substitution_spans(source);
        let escaped = backtick_escaped_parameters(source, &backtick_spans);

        assert_eq!(escaped.len(), 1);
        assert!(escaped[0].standalone_command_name);
    }

    fn span_for_text(source: &str, text: &str) -> Span {
        let start_offset = source.find(text).expect("expected text");
        let end_offset = start_offset + text.len();
        Span::from_positions(
            position_at_offset(source, start_offset).expect("expected start position"),
            position_at_offset(source, end_offset).expect("expected end position"),
        )
    }

    #[test]
    fn backtick_double_escaped_parameter_spans_track_quoted_templates() {
        let source =
            r#"`echo "foreach dir {puts \\$dir} literal \\\\$literal"` `echo "plain $missing"`"#;
        let backtick_spans = backtick_substitution_spans(source);
        let escaped = backtick_double_escaped_parameter_spans(source, &backtick_spans);

        assert_eq!(
            escaped
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$dir"]
        );
    }

    #[test]
    fn word_positional_at_splat_span_in_source_tracks_operation_forms() {
        let source = "printf '%s\\n' \"${@:-fallback}\" \"$@\" \"\\$@\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert_eq!(
            word_positional_at_splat_span_in_source(&command.args[1], source)
                .expect("expected positional span")
                .slice(source),
            "${@:-fallback}"
        );
        assert_eq!(
            word_positional_at_splat_span_in_source(&command.args[2], source)
                .expect("expected positional span")
                .slice(source),
            "$@"
        );
        assert!(word_positional_at_splat_span_in_source(&command.args[3], source).is_none());
    }
}
