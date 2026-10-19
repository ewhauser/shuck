use super::*;

pub(crate) fn render_var_ref_to_buf(reference: &VarRef, source: &str, rendered: &mut String) {
    rendered.push_str(reference.name.as_str());
    if let Some(subscript) = &reference.subscript {
        rendered.push('[');
        render_subscript_to_buf(subscript, source, rendered);
        rendered.push(']');
    }
}

pub(crate) fn render_subscript_to_buf(subscript: &Subscript, source: &str, rendered: &mut String) {
    if let Some(selector) = subscript.selector() {
        rendered.push(selector.as_char());
        return;
    }

    render_source_text_to_buf(subscript.syntax_source_text(), source, rendered);
}

pub(crate) fn trim_unescaped_trailing_whitespace(text: &str) -> &str {
    let mut end = text.len();
    while end > 0 {
        let Some((whitespace_start, ch)) = text[..end].char_indices().next_back() else {
            break;
        };
        if !ch.is_whitespace() {
            break;
        }

        let backslash_count = text.as_bytes()[..whitespace_start]
            .iter()
            .rev()
            .take_while(|byte| **byte == b'\\')
            .count();
        if backslash_count % 2 == 1 {
            break;
        }

        end = whitespace_start;
    }

    &text[..end]
}

pub(crate) fn render_source_text_to_buf(text: &SourceText, source: &str, rendered: &mut String) {
    if !text.is_source_backed() || text.span().end.offset() <= source.len() {
        rendered.push_str(text.slice(source));
    }
}

pub(crate) fn render_background_operator(operator: BackgroundOperator) -> &'static str {
    match operator {
        BackgroundOperator::Plain => "&",
        BackgroundOperator::Pipe => "&|",
        BackgroundOperator::Bang => "&!",
    }
}

pub(crate) fn case_terminator(terminator: CaseTerminator) -> &'static str {
    match terminator {
        CaseTerminator::Break => ";;",
        CaseTerminator::FallThrough => ";&",
        CaseTerminator::Continue => ";;&",
        CaseTerminator::ContinueMatching => ";|",
    }
}

pub(crate) fn binary_operator(operator: &shuck_ast::BinaryOp) -> &'static str {
    match operator {
        shuck_ast::BinaryOp::And => "&&",
        shuck_ast::BinaryOp::Or => "||",
        shuck_ast::BinaryOp::Pipe => "|",
        shuck_ast::BinaryOp::PipeAll => "|&",
    }
}

pub(crate) fn slice_span(source: &str, span: Option<Span>) -> &str {
    span.and_then(|span| source.get(span.start.offset()..span.end.offset()))
        .unwrap_or("")
}

pub(crate) fn extend_heredoc_body_span(span: Span, source: &str) -> Span {
    // Recovered parses can leave heredoc spans past the end of the source;
    // clamp instead of slicing out of bounds.
    let start = span.start.offset();
    let mut end = span.end.offset().max(start);
    while end < source.len() {
        let byte = source.as_bytes()[end];
        end += 1;
        if byte == b'\n' {
            break;
        }
    }
    let Some(body) = source.get(start..end) else {
        return span;
    };
    let end_position = span.start.advanced_by(body);
    Span::from_positions(span.start, end_position)
}

#[cfg(test)]
mod tests {
    use shuck_ast::Position;

    use super::*;

    #[test]
    fn extend_heredoc_body_span_tolerates_spans_past_eof() {
        let source = "cat <<F\nbody";
        let at = |offset: usize| Position::new().advanced_by(&"x".repeat(offset));

        let span = Span::from_positions(at(8), at(source.len() + 5));
        assert_eq!(extend_heredoc_body_span(span, source), span);

        let span = Span::from_positions(at(source.len() + 1), at(source.len() + 1));
        assert_eq!(extend_heredoc_body_span(span, source), span);
    }
}
