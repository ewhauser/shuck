use super::*;

pub fn collect_command_substitution_part_spans_in_source(
    word: &Word,
    locator: Locator<'_>,
    spans: &mut Vec<Span>,
) {
    collect_command_substitution_spans(&word.parts, spans);
    normalize_command_substitution_spans(spans, locator);
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

pub fn collect_unquoted_command_substitution_part_spans_in_source(
    word: &Word,
    locator: Locator<'_>,
    spans: &mut Vec<Span>,
) {
    collect_unquoted_command_substitution_spans(&word.parts, false, spans);
    normalize_command_substitution_spans(spans, locator);
}

pub fn collect_unquoted_dollar_paren_command_substitution_part_spans_in_source(
    word: &Word,
    locator: Locator<'_>,
    spans: &mut Vec<Span>,
) {
    collect_unquoted_dollar_paren_command_substitution_spans(&word.parts, false, spans);
    normalize_command_substitution_spans(spans, locator);
}

pub(crate) fn collect_command_substitution_spans(parts: &[WordPartNode], spans: &mut Vec<Span>) {
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

pub(crate) fn collect_arithmetic_expansion_spans(parts: &[WordPartNode], spans: &mut Vec<Span>) {
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

pub(crate) fn collect_parenthesized_arithmetic_expansion_spans(
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

pub(crate) fn collect_unquoted_command_substitution_spans(
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

pub(crate) fn collect_unquoted_dollar_paren_command_substitution_spans(
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

pub(crate) fn normalize_command_substitution_span(span: Span, locator: Locator<'_>) -> Span {
    let source = locator.source();
    let text = span.slice(source);
    if text.starts_with("$(")
        && !text.ends_with(')')
        && let Some(normalized) = widen_dollar_paren_command_substitution_span(span, locator)
    {
        return normalized;
    }

    if text.starts_with('`')
        && !text.ends_with('`')
        && let Some(normalized) = widen_backtick_command_substitution_span(span, locator)
    {
        return normalized;
    }

    span
}

pub(crate) fn normalize_command_substitution_spans(spans: &mut [Span], locator: Locator<'_>) {
    for span in spans {
        *span = normalize_command_substitution_span(*span, locator);
    }
}

pub(crate) fn widen_dollar_paren_command_substitution_span(
    span: Span,
    locator: Locator<'_>,
) -> Option<Span> {
    let source = locator.source();
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
                        let start = locator.position_at_offset(span.start.offset)?;
                        let end = locator.position_at_offset(index)?;
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
                    let start = locator.position_at_offset(span.start.offset)?;
                    let end = locator.position_at_offset(index)?;
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

pub(crate) fn widen_backtick_command_substitution_span(
    span: Span,
    locator: Locator<'_>,
) -> Option<Span> {
    let source = locator.source();
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
                let start = locator.position_at_offset(span.start.offset)?;
                let end = locator.position_at_offset(index)?;
                return Some(Span::from_positions(start, end));
            }
            _ => index += 1,
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use shuck_ast::{Span, Word};
    use shuck_indexer::LineIndex;
    use shuck_parser::parser::Parser;

    use super::{
        collect_command_substitution_spans,
        collect_unquoted_dollar_paren_command_substitution_part_spans_in_source,
    };
    use crate::Locator;

    fn command_substitution_part_spans(word: &Word) -> Vec<Span> {
        let mut spans = Vec::new();
        collect_command_substitution_spans(&word.parts, &mut spans);
        spans
    }

    fn unquoted_dollar_paren_command_substitution_part_spans_in_source(
        word: &Word,
        source: &str,
    ) -> Vec<Span> {
        let line_index = LineIndex::new(source);
        let locator = Locator::new(source, &line_index);
        let mut spans = Vec::new();
        collect_unquoted_dollar_paren_command_substitution_part_spans_in_source(
            word, locator, &mut spans,
        );
        spans
    }

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
}
