use super::*;

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

pub fn word_use_replacement_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_use_replacement_spans(&word.parts, &mut spans);
    spans
}

pub(super) fn collect_arithmetic_expansion_spans(parts: &[WordPartNode], spans: &mut Vec<Span>) {
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

pub(super) fn collect_parenthesized_arithmetic_expansion_spans(
    parts: &[WordPartNode],
    spans: &mut Vec<Span>,
) {
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

pub(super) fn collect_expansion_spans(parts: &[WordPartNode], spans: &mut Vec<Span>) {
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

pub(super) fn collect_scalar_expansion_spans(
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

pub(super) fn collect_use_replacement_spans(parts: &[WordPartNode], spans: &mut Vec<Span>) {
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

pub(super) fn collect_literal_scan_exclusions(parts: &[WordPartNode], excluded: &mut Vec<Span>) {
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

pub(super) fn paren_expansion_len(text: &str) -> Option<usize> {
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

pub(super) fn word_has_only_literal_parts(parts: &[WordPartNode]) -> bool {
    parts
        .iter()
        .all(|part| matches!(part.kind, WordPart::Literal(_)))
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;
    #[allow(unused_imports)]
    use crate::facts::word_spans::*;
    #[allow(unused_imports)]
    use shuck_ast::Span;
    #[allow(unused_imports)]
    use shuck_parser::parser::Parser;

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
}
