use shuck_ast::{
    Assignment, BinaryCommand, BourneParameterExpansion, ConditionalExpr, ParameterExpansion,
    ParameterExpansionSyntax, Pattern, PatternGroupKind, PatternPart, Redirect, Span, VarRef,
    Word, WordPart, WordPartNode, ZshExpansionTarget,
};

pub fn assignment_name_span(assignment: &Assignment) -> Span {
    assignment.target.name_span
}

pub fn binary_operator_span(command: &BinaryCommand) -> Span {
    command.op_span
}

pub fn redirect_target_span(redirect: &Redirect) -> Span {
    redirect
        .word_target()
        .expect("redirect_target_span called on heredoc redirect")
        .span
}

pub fn heredoc_delimiter_span(redirect: &Redirect) -> Span {
    redirect
        .heredoc()
        .expect("heredoc_delimiter_span called on non-heredoc redirect")
        .delimiter
        .span
}

pub fn heredoc_body_span(redirect: &Redirect) -> Span {
    redirect
        .heredoc()
        .expect("heredoc_body_span called on non-heredoc redirect")
        .body
        .span
}

pub fn command_substitution_part_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_command_substitution_spans(&word.parts, &mut spans);
    spans
}

pub fn unquoted_command_substitution_part_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_unquoted_command_substitution_spans(&word.parts, false, &mut spans);
    spans
}

pub fn array_expansion_part_spans(word: &Word, _source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_array_expansion_spans(&word.parts, false, false, &mut spans);
    spans
}

pub fn unquoted_array_expansion_part_spans(word: &Word, _source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_array_expansion_spans(&word.parts, false, true, &mut spans);
    spans
}

pub fn expansion_part_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_expansion_spans(&word.parts, &mut spans);
    spans
}

pub fn scalar_expansion_part_spans(word: &Word, _source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_scalar_expansion_spans(&word.parts, &mut spans);
    spans
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

pub fn word_literal_part_spans_excluding_parameter_operator_tails(
    word: &Word,
    source: &str,
) -> Vec<Span> {
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
        })
        .collect()
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
    scan_span_excluding(word.span, &excluded, source)
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
            (unquoted_array_spans.contains(&span)
                && matches!(part, WordPart::Variable(name) if name.as_str() == "*"))
            .then_some(span)
        })
        .collect()
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
    word_extglob_span_from_parts(&word.parts, source).or_else(|| {
        (!word.has_quoted_parts()
            && word
                .parts
                .iter()
                .all(|part| matches!(part.kind, WordPart::Literal(_)))
            && text_looks_like_extglob(word.span.slice(source)))
        .then_some(word.span)
    })
}

pub fn word_exactly_one_extglob_span(word: &Word, source: &str) -> Option<Span> {
    word_exactly_one_extglob_span_from_parts(&word.parts, source).or_else(|| {
        (!word.has_quoted_parts()
            && word_has_only_literal_parts(&word.parts)
            && text_looks_like_exactly_one_extglob(word.span.slice(source)))
        .then_some(word.span)
    })
}

pub fn conditional_exactly_one_extglob_span(
    expression: &ConditionalExpr,
    source: &str,
) -> Option<Span> {
    match expression {
        ConditionalExpr::Binary(expr) => {
            conditional_exactly_one_extglob_span(&expr.left, source)
                .or_else(|| conditional_exactly_one_extglob_span(&expr.right, source))
        }
        ConditionalExpr::Unary(expr) => conditional_exactly_one_extglob_span(&expr.expr, source),
        ConditionalExpr::Parenthesized(expr) => {
            conditional_exactly_one_extglob_span(&expr.expr, source)
        }
        ConditionalExpr::Pattern(pattern) => pattern_exactly_one_extglob_span(pattern, source),
        ConditionalExpr::Word(_) | ConditionalExpr::Regex(_) | ConditionalExpr::VarRef(_) => None,
    }
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
            WordPart::ArraySlice { .. } | WordPart::ArrayIndices(_)
                if !quoted || !only_unquoted =>
            {
                spans.push(part.span);
            }
            _ => {}
        }
    }
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

fn collect_scalar_expansion_spans(parts: &[WordPartNode], spans: &mut Vec<Span>) {
    for part in parts {
        match &part.kind {
            WordPart::Literal(_) | WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => collect_scalar_expansion_spans(parts, spans),
            WordPart::ZshQualifiedGlob(_) => {}
            WordPart::CommandSubstitution { .. } | WordPart::ProcessSubstitution { .. } => {}
            WordPart::Parameter(parameter) => {
                if parameter_is_scalar_like(parameter) {
                    spans.push(part.span);
                }
            }
            WordPart::Variable(name) if matches!(name.as_str(), "@" | "*") => {}
            WordPart::Variable(_)
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayLength(_)
            | WordPart::Substring { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::Transformation { .. } => spans.push(part.span),
            WordPart::ArrayAccess(reference) => {
                if !reference.has_array_selector() {
                    spans.push(part.span);
                }
            }
            WordPart::ArrayIndices(_) | WordPart::ArraySlice { .. } => {}
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

fn scan_span_excluding(span: Span, excluded: &[Span], source: &str) -> Vec<Span> {
    if excluded.is_empty() {
        return vec![span];
    }

    let mut spans = Vec::new();
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

    spans
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

fn word_extglob_span_from_parts(parts: &[WordPartNode], source: &str) -> Option<Span> {
    for part in parts {
        match &part.kind {
            WordPart::Literal(_) => {
                if text_looks_like_extglob(part.span.slice(source)) {
                    return Some(part.span);
                }
            }
            WordPart::DoubleQuoted { .. } | WordPart::SingleQuoted { .. } => {}
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
            | WordPart::Transformation { .. } => {}
        }
    }

    None
}

fn word_exactly_one_extglob_span_from_parts(
    parts: &[WordPartNode],
    source: &str,
) -> Option<Span> {
    for part in parts {
        match &part.kind {
            WordPart::Literal(_) => {
                if text_looks_like_exactly_one_extglob(part.span.slice(source)) {
                    return Some(part.span);
                }
            }
            WordPart::DoubleQuoted { .. } | WordPart::SingleQuoted { .. } => {}
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
            | WordPart::Transformation { .. } => {}
        }
    }

    None
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

fn word_has_only_literal_parts(parts: &[WordPartNode]) -> bool {
    parts
        .iter()
        .all(|part| matches!(part.kind, WordPart::Literal(_)))
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

fn text_looks_like_extglob(text: &str) -> bool {
    let bytes = text.as_bytes();
    if text_has_parenthesized_alternation(bytes) {
        return true;
    }

    let mut index = 0usize;

    while index + 1 < bytes.len() {
        if !is_extglob_operator(bytes[index])
            || bytes[index + 1] != b'('
            || byte_is_backslash_escaped(bytes, index)
        {
            index += 1;
            continue;
        }

        return matching_group_end(bytes, index + 1).is_some();
    }

    false
}

fn text_looks_like_exactly_one_extglob(text: &str) -> bool {
    let bytes = text.as_bytes();

    let mut index = 0usize;
    while index + 1 < bytes.len() {
        if bytes[index] != b'@'
            || bytes[index + 1] != b'('
            || byte_is_backslash_escaped(bytes, index)
        {
            index += 1;
            continue;
        }

        return matching_group_end(bytes, index + 1).is_some();
    }

    false
}

fn text_has_parenthesized_alternation(bytes: &[u8]) -> bool {
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
            return true;
        }

        index = close + 1;
    }

    false
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
            _ => false,
        },
        ParameterExpansionSyntax::Zsh(_) => false,
    }
}

fn parameter_is_scalar_like(parameter: &ParameterExpansion) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference } => !reference.has_array_selector(),
            BourneParameterExpansion::Length { .. }
            | BourneParameterExpansion::Indirect { .. }
            | BourneParameterExpansion::PrefixMatch { .. }
            | BourneParameterExpansion::Operation { .. }
            | BourneParameterExpansion::Transformation { .. } => true,
            BourneParameterExpansion::Indices { .. } => false,
            BourneParameterExpansion::Slice { reference, .. } => !reference.has_array_selector(),
        },
        ParameterExpansionSyntax::Zsh(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use shuck_parser::parser::Parser;

    use super::{
        array_expansion_part_spans, command_substitution_part_spans, scalar_expansion_part_spans,
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
        let source = "printf '%s\\n' ${arr[@]} ${arr[0]}\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = array_expansion_part_spans(&command.args[1], source);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].slice(source), "${arr[@]}");
    }

    #[test]
    fn scalar_expansion_spans_ignore_array_splats_and_command_substitutions() {
        let source = "printf '%s\\n' prefix${name}suffix ${arr[@]} ${arr[0]} $(date)\n";
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
}
