use shuck_ast::{
    Assignment, CommandListItem, Position, Redirect, Span, TextRange, TextSize, Word, WordPart,
};
use shuck_indexer::Indexer;

pub fn assignment_name_span(assignment: &Assignment) -> Span {
    assignment.name_span
}

pub fn list_item_operator_span(item: &CommandListItem) -> Span {
    item.operator_span
}

pub fn redirect_target_span(redirect: &Redirect) -> Span {
    redirect.target.span
}

pub fn command_substitution_part_spans(word: &Word) -> Vec<Span> {
    word.parts_with_spans()
        .filter_map(|(part, span)| match part {
            WordPart::CommandSubstitution(_) => Some(span),
            _ => None,
        })
        .collect()
}

pub fn array_expansion_part_spans(word: &Word, source: &str) -> Vec<Span> {
    word.parts_with_spans()
        .filter_map(|(part, span)| match part {
            WordPart::ArrayAccess { index, .. } if matches!(index.slice(source), "@" | "*") => {
                Some(span)
            }
            WordPart::ArraySlice { .. } | WordPart::ArrayIndices(_) => Some(span),
            _ => None,
        })
        .collect()
}

pub fn expansion_part_spans(word: &Word) -> Vec<Span> {
    word.parts_with_spans()
        .filter_map(|(part, span)| match part {
            WordPart::Literal(_) => None,
            WordPart::Variable(_)
            | WordPart::CommandSubstitution(_)
            | WordPart::ArithmeticExpansion(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayAccess { .. }
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::Substring { .. }
            | WordPart::ArraySlice { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::PrefixMatch(_)
            | WordPart::ProcessSubstitution { .. }
            | WordPart::Transformation { .. } => Some(span),
        })
        .collect()
}

pub fn single_quoted_region_span(indexer: &Indexer, span: Span) -> Span {
    let offset = TextSize::new(span.start.offset as u32);
    let Some(range) = indexer.region_index().single_quoted_range_at(offset) else {
        return span;
    };

    text_range_span(indexer, range)
}

pub fn backtick_fragment_spans(word: &Word, source: &str) -> Vec<Span> {
    let text = word.span.slice(source);
    let mut spans = Vec::new();
    let mut cursor = word.span.start;
    let mut in_single_quotes = false;
    let mut escaped = false;
    let mut fragment_start = None;

    for ch in text.chars() {
        let current = cursor;
        cursor.advance(ch);

        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' if !in_single_quotes => escaped = true,
            '\'' => in_single_quotes = !in_single_quotes,
            '`' if !in_single_quotes => {
                if let Some(start) = fragment_start.take() {
                    spans.push(Span::from_positions(start, cursor));
                } else {
                    fragment_start = Some(current);
                }
            }
            _ => {}
        }
    }

    spans
}

pub fn position_for_offset(indexer: &Indexer, offset: TextSize) -> Position {
    let line = indexer.line_index().line_number(offset);
    let line_start = indexer
        .line_index()
        .line_start(line)
        .unwrap_or_else(|| TextSize::new(0));

    Position {
        line,
        column: usize::from(offset) - usize::from(line_start) + 1,
        offset: usize::from(offset),
    }
}

pub fn text_range_span(indexer: &Indexer, range: TextRange) -> Span {
    Span::from_positions(
        position_for_offset(indexer, range.start()),
        position_for_offset(indexer, range.end()),
    )
}

#[cfg(test)]
mod tests {
    use shuck_ast::{Position, Span, Word};
    use shuck_parser::parser::Parser;

    use super::{
        array_expansion_part_spans, backtick_fragment_spans, command_substitution_part_spans,
    };

    #[test]
    fn command_substitution_spans_use_inner_part_ranges() {
        let source = "printf '%s\\n' prefix$(date)suffix\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.script.commands[0];
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
        let command = &output.script.commands[0];
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = array_expansion_part_spans(&command.args[1], source);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].slice(source), "${arr[@]}");
    }

    #[test]
    fn backtick_fragment_spans_find_exact_pairs() {
        let word = Word::quoted_literal_with_span(
            "today is `date` and `uname`",
            Span::from_positions(
                Position {
                    line: 1,
                    column: 1,
                    offset: 0,
                },
                Position {
                    line: 1,
                    column: 28,
                    offset: 27,
                },
            ),
        );
        let source = "today is `date` and `uname`";

        let spans = backtick_fragment_spans(&word, source);
        assert_eq!(
            spans
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["`date`", "`uname`"]
        );
    }
}
