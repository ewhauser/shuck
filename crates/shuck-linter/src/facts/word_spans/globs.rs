use super::*;

pub fn word_unquoted_glob_pattern_spans(word: &Word, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_unquoted_glob_pattern_spans(&word.parts, source, false, &mut spans);
    spans
}

pub fn word_active_glob_pattern_spans(
    word: &Word,
    source: &str,
    pathname_behavior: PathnameExpansionBehavior,
    pattern_behavior: GlobPatternBehavior,
) -> Vec<Span> {
    if !pathname_behavior.literal_globs_can_expand() {
        return Vec::new();
    }

    let mut spans = Vec::new();
    collect_active_glob_pattern_spans(&word.parts, source, false, pattern_behavior, &mut spans);
    spans.sort_by_key(|span| (span.start.offset, span.end.offset));
    spans.dedup();
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

pub fn word_active_glob_pattern_spans_outside_brace_expansion(
    word: &Word,
    source: &str,
    pathname_behavior: PathnameExpansionBehavior,
    pattern_behavior: GlobPatternBehavior,
) -> Vec<Span> {
    let active_brace_spans = word
        .brace_syntax()
        .iter()
        .copied()
        .filter(|brace| brace.expands())
        .map(|brace| brace.span)
        .collect::<Vec<_>>();

    if active_brace_spans.is_empty() {
        return word_active_glob_pattern_spans(word, source, pathname_behavior, pattern_behavior);
    }

    word_active_glob_pattern_spans(word, source, pathname_behavior, pattern_behavior)
        .into_iter()
        .filter(|glob_span| {
            !active_brace_spans.iter().any(|brace_span| {
                brace_span.start.offset <= glob_span.start.offset
                    && glob_span.end.offset <= brace_span.end.offset
            })
        })
        .collect()
}

pub fn word_starts_with_active_glob_group_operator(
    word: &Word,
    source: &str,
    pathname_behavior: PathnameExpansionBehavior,
    pattern_behavior: GlobPatternBehavior,
) -> bool {
    if !pathname_behavior.literal_globs_can_expand()
        || !pattern_operator_may_be_active(pattern_behavior.ksh_glob())
    {
        return false;
    }

    let bytes = word.span.slice(source).as_bytes();
    matches!(
        find_ksh_glob_group_bounds(bytes).first().copied(),
        Some((0, _))
    )
}

pub fn word_suspicious_bracket_glob_spans(word: &Word, source: &str) -> Vec<Span> {
    word_unquoted_glob_pattern_spans(word, source)
        .into_iter()
        .filter(|span| suspicious_bracket_glob_text(span.slice(source)))
        .collect()
}

pub fn word_suspicious_brace_character_class_spans(word: &Word, source: &str) -> Vec<Span> {
    let mut spans = word
        .brace_syntax()
        .iter()
        .copied()
        .filter(|brace| {
            matches!(
                brace.kind,
                shuck_ast::BraceSyntaxKind::Expansion(
                    shuck_ast::BraceExpansionKind::CharacterClass
                ) | shuck_ast::BraceSyntaxKind::Literal
            ) && matches!(brace.quote_context, shuck_ast::BraceQuoteContext::Unquoted)
                && suspicious_brace_character_class_text(brace.span.slice(source))
        })
        .map(|brace| brace.span)
        .collect::<Vec<_>>();
    collect_literal_suspicious_brace_character_class_spans(word, source, &mut spans);
    spans.sort_unstable_by_key(|span| (span.start.offset, span.end.offset));
    spans.dedup();
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

pub(crate) fn collect_conditional_suspicious_bracket_glob_spans(
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

pub(crate) fn collect_pattern_suspicious_bracket_glob_spans(
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

pub(crate) fn collect_unquoted_glob_pattern_spans(
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

pub(crate) fn collect_active_glob_pattern_spans(
    parts: &[WordPartNode],
    source: &str,
    in_double_quotes: bool,
    pattern_behavior: GlobPatternBehavior,
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
        spans.extend(active_literal_glob_pattern_spans(
            combined_span,
            source,
            pattern_behavior,
        ));
    };

    for (index, part) in parts.iter().enumerate() {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                flush_literal_run(&mut literal_run_start, &mut literal_run_end, spans);
                collect_active_glob_pattern_spans(parts, source, true, pattern_behavior, spans)
            }
            WordPart::Literal(_)
                if !in_double_quotes
                    && !literal_part_is_parameter_operator_tail(parts, index, source) =>
            {
                literal_run_start.get_or_insert(index);
                literal_run_end = Some(index);
            }
            WordPart::ZshQualifiedGlob(glob) if !in_double_quotes => {
                flush_literal_run(&mut literal_run_start, &mut literal_run_end, spans);
                spans.extend(zsh_qualified_glob_active_pattern_spans(
                    glob,
                    source,
                    pattern_behavior,
                ));
            }
            WordPart::Literal(_)
            | WordPart::ZshQualifiedGlob(_)
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
            | WordPart::Transformation { .. } => {
                flush_literal_run(&mut literal_run_start, &mut literal_run_end, spans);
            }
        }
    }

    flush_literal_run(&mut literal_run_start, &mut literal_run_end, spans);
}

pub(crate) fn literal_glob_pattern_spans(span: Span, source: &str) -> Vec<Span> {
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

pub(crate) fn active_literal_glob_pattern_spans(
    span: Span,
    source: &str,
    pattern_behavior: GlobPatternBehavior,
) -> Vec<Span> {
    let text = span.slice(source);
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut covered = Vec::<(usize, usize)>::new();

    if pattern_operator_may_be_active(pattern_behavior.ksh_glob()) {
        for (start, end) in find_ksh_glob_group_bounds(bytes) {
            spans.push(span_within_literal(span, source, start, end + 1));
            covered.push((start, end));
        }
    }

    if pattern_operator_may_be_active(pattern_behavior.extended_glob()) {
        for (start, end) in find_zsh_extended_glob_operator_bounds(bytes, &covered) {
            if covered
                .iter()
                .any(|(covered_start, covered_end)| *covered_start <= start && end <= *covered_end)
            {
                continue;
            }
            spans.push(span_within_literal(span, source, start, end + 1));
            covered.push((start, end));
        }
    }

    let basic_spans = literal_glob_pattern_spans(span, source)
        .into_iter()
        .filter(|glob_span| !span_is_within_any(*glob_span, &spans))
        .collect::<Vec<_>>();
    spans.extend(basic_spans);
    spans.sort_by_key(|span| (span.start.offset, span.end.offset));
    spans.dedup();
    spans
}

pub(crate) fn find_ksh_glob_group_bounds(bytes: &[u8]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
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
            spans.push((index, close));
            index = close + 1;
        } else {
            index += 1;
        }
    }

    spans
}

pub(crate) fn find_zsh_extended_glob_operator_bounds(
    bytes: &[u8],
    active_group_bounds: &[(usize, usize)],
) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut index = 0usize;

    while index < bytes.len() {
        if byte_is_backslash_escaped(bytes, index) {
            index += 1;
            continue;
        }

        match bytes[index] {
            b'#' | b'^' => spans.push((index, index)),
            b'~' if zsh_exclusion_operator_has_pattern_operand(
                bytes,
                index,
                active_group_bounds,
            ) =>
            {
                spans.push((index, index));
            }
            _ => {}
        }

        index += 1;
    }

    spans
}

fn zsh_exclusion_operator_has_pattern_operand(
    bytes: &[u8],
    tilde_index: usize,
    active_group_bounds: &[(usize, usize)],
) -> bool {
    literal_range_has_active_glob_syntax(bytes, 0, tilde_index, active_group_bounds)
        || literal_range_has_active_glob_syntax(
            bytes,
            tilde_index + 1,
            bytes.len(),
            active_group_bounds,
        )
}

pub(crate) fn zsh_qualified_glob_active_pattern_spans(
    glob: &ZshQualifiedGlob,
    source: &str,
    pattern_behavior: GlobPatternBehavior,
) -> Vec<Span> {
    if zsh_qualified_glob_has_control_syntax(glob) {
        vec![glob.span]
    } else {
        active_literal_glob_pattern_spans(glob.span, source, pattern_behavior)
    }
}

pub(crate) fn zsh_qualified_glob_has_control_syntax(glob: &ZshQualifiedGlob) -> bool {
    glob.qualifiers.is_some()
        || glob
            .segments
            .iter()
            .any(|segment| matches!(segment, ZshGlobSegment::InlineControl(_)))
}

fn literal_range_has_active_glob_syntax(
    bytes: &[u8],
    start: usize,
    end: usize,
    active_group_bounds: &[(usize, usize)],
) -> bool {
    if active_group_bounds
        .iter()
        .any(|(group_start, group_end)| start <= *group_start && *group_end < end)
    {
        return true;
    }

    let mut index = start;
    while index < end {
        if byte_is_backslash_escaped(bytes, index) {
            index += 1;
            continue;
        }

        match bytes[index] {
            b'*' | b'?' | b'#' | b'^' => return true,
            b'[' => {
                let mut close = index + 1;
                while close < end {
                    if let Some(named_end) = bracket_glob_named_class_end(bytes, close, end) {
                        close = named_end;
                        continue;
                    }
                    if bytes[close] == b'\\' {
                        close = (close + 2).min(end);
                        continue;
                    }
                    if bytes[close] == b']' {
                        return true;
                    }
                    close += 1;
                }
                index += 1;
            }
            _ => index += 1,
        }
    }

    false
}

fn pattern_operator_may_be_active(behavior: PatternOperatorBehavior) -> bool {
    !matches!(behavior, PatternOperatorBehavior::Disabled)
}

fn span_is_within_any(span: Span, hosts: &[Span]) -> bool {
    hosts
        .iter()
        .any(|host| host.start.offset <= span.start.offset && span.end.offset <= host.end.offset)
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

fn suspicious_brace_character_class_text(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() < 3 || bytes[0] != b'{' || *bytes.last().unwrap_or(&b'\0') != b'}' {
        return false;
    }

    let mut bracket_text = String::with_capacity(text.len());
    bracket_text.push('[');
    bracket_text.push_str(&text[1..text.len() - 1]);
    bracket_text.push(']');
    suspicious_bracket_glob_text(&bracket_text)
}

fn collect_literal_suspicious_brace_character_class_spans(
    word: &Word,
    source: &str,
    out: &mut Vec<Span>,
) {
    for part in &word.parts {
        if !matches!(part.kind, WordPart::Literal(_)) {
            continue;
        }

        let text = part.span.slice(source);
        let bytes = text.as_bytes();
        let mut index = 0usize;
        while index < bytes.len() {
            if bytes[index] != b'{' || byte_is_backslash_escaped(bytes, index) {
                index += 1;
                continue;
            }

            let mut close = index + 1;
            while close < bytes.len() {
                if bytes[close] == b'}' && !byte_is_backslash_escaped(bytes, close) {
                    let span = span_within_literal(part.span, source, index, close + 1);
                    if suspicious_brace_character_class_text(span.slice(source)) {
                        out.push(span);
                    }
                    index = close + 1;
                    break;
                }
                close += 1;
            }

            if close >= bytes.len() {
                index += 1;
            }
        }
    }
}

pub(crate) fn bracket_glob_is_named_class_without_outer_brackets(bytes: &[u8]) -> bool {
    if bytes.len() < 5 {
        return false;
    }

    let kind = bytes[1];
    if !matches!(kind, b':' | b'.' | b'=') {
        return false;
    }

    bytes[bytes.len() - 2] == kind
}

pub(crate) fn bracket_glob_named_class_end(
    bytes: &[u8],
    start: usize,
    limit: usize,
) -> Option<usize> {
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

pub(crate) fn hyphen_is_range_separator(
    bytes: &[u8],
    index: usize,
    start: usize,
    end: usize,
) -> bool {
    if bytes[index] != b'-' || index == start || index + 1 >= end {
        return false;
    }

    if bracket_glob_named_class_end(bytes, index + 1, bytes.len()).is_some() {
        return false;
    }

    true
}

pub(crate) fn span_within_literal(span: Span, source: &str, start: usize, end: usize) -> Span {
    let start_pos = span
        .start
        .advanced_by(&source[span.start.offset..span.start.offset + start]);
    let end_pos = span
        .start
        .advanced_by(&source[span.start.offset..span.start.offset + end]);
    Span::from_positions(start_pos, end_pos)
}

pub(crate) fn pattern_extglob_span(pattern: &Pattern, source: &str) -> Option<Span> {
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

pub(crate) fn pattern_array_subscript_span(pattern: &Pattern, source: &str) -> Option<Span> {
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

pub(crate) fn word_array_subscript_span_from_parts(
    parts: &[WordPartNode],
    source: &str,
) -> Option<Span> {
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

pub(crate) fn collect_unbraced_variable_before_bracket_spans(
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

pub(crate) fn is_named_shell_variable(name: &str) -> bool {
    let bytes = name.as_bytes();
    let Some((&first, rest)) = bytes.split_first() else {
        return false;
    };

    is_name_start(first) && rest.iter().copied().all(is_name_continue)
}

pub(crate) fn unbraced_variable_dollar_span(part: &WordPartNode, source: &str) -> Span {
    let raw = part.span.slice(source);
    let dollar_offset = raw.find('$').unwrap_or(0);
    Span::at(part.span.start.advanced_by(&raw[..dollar_offset]))
}

pub(crate) fn variable_part_uses_braces(part: &WordPartNode, source: &str) -> bool {
    let raw = part.span.slice(source);
    raw.find('$')
        .and_then(|offset| raw.as_bytes().get(offset + 1))
        .is_some_and(|next| *next == b'{')
}

pub(crate) fn parameter_array_subscript_span(parameter: &ParameterExpansion) -> Option<Span> {
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

pub(crate) fn var_ref_subscript_span(reference: &VarRef) -> Option<Span> {
    reference
        .subscript
        .as_ref()
        .filter(|subscript| subscript.selector().is_none())
        .map(|_| reference.span)
}

pub(crate) fn word_surface_bytes(
    word: &Word,
    source: &str,
) -> Option<(Vec<u8>, Vec<Option<usize>>)> {
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

pub(crate) fn word_extglob_span_from_literal_parts(
    parts: &[WordPartNode],
    source: &str,
) -> Option<Span> {
    for part in parts {
        if matches!(part.kind, WordPart::Literal(_))
            && find_extglob_bounds(part.span.slice(source).as_bytes()).is_some()
        {
            return Some(part.span);
        }
    }

    None
}

pub(crate) fn word_exactly_one_extglob_span_from_literal_parts(
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

pub(crate) fn word_caret_negated_bracket_spans_from_literal_parts(
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

pub(crate) fn word_surface_span_from_bounds(
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

pub(crate) fn word_has_only_literal_parts(parts: &[WordPartNode]) -> bool {
    parts
        .iter()
        .all(|part| matches!(part.kind, WordPart::Literal(_)))
}

pub(crate) fn text_has_variable_subscript(text: &str) -> bool {
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

pub(crate) fn find_parenthesized_alternation_bounds(bytes: &[u8]) -> Option<(usize, usize)> {
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

pub(crate) fn find_extglob_bounds(bytes: &[u8]) -> Option<(usize, usize)> {
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

pub(crate) fn find_exactly_one_extglob_bounds(bytes: &[u8]) -> Option<(usize, usize)> {
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

pub(crate) fn find_caret_negated_bracket_bounds(bytes: &[u8]) -> Vec<(usize, usize)> {
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

pub(crate) fn matching_group_end(bytes: &[u8], open_index: usize) -> Option<usize> {
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

pub(crate) fn byte_is_backslash_escaped(bytes: &[u8], index: usize) -> bool {
    let mut cursor = index;
    let mut backslashes = 0usize;

    while cursor > 0 && bytes[cursor - 1] == b'\\' {
        backslashes += 1;
        cursor -= 1;
    }

    backslashes % 2 == 1
}

pub(crate) fn is_extglob_operator(byte: u8) -> bool {
    matches!(byte, b'@' | b'?' | b'+' | b'*' | b'!')
}

pub(crate) fn is_name_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

pub(crate) fn is_name_continue(byte: u8) -> bool {
    is_name_start(byte) || byte.is_ascii_digit()
}

#[cfg(test)]
mod tests {
    use shuck_ast::Span;
    use shuck_parser::parser::Parser;

    use super::{
        find_extglob_bounds, word_caret_negated_bracket_spans, word_exactly_one_extglob_span,
        word_starts_with_extglob, word_suspicious_bracket_glob_spans,
        word_unquoted_glob_pattern_spans, word_unquoted_glob_pattern_spans_outside_brace_expansion,
    };

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
}
