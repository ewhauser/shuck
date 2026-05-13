use shuck_ast::{Position, Span};

pub(super) fn case_item_deletion_span(item: &shuck_ast::CaseItem, source: &str) -> Option<Span> {
    let first = item.patterns.first()?.span;
    let end = item.terminator_span.unwrap_or(item.body.span).end;
    let mut start_offset = first.start.offset;
    while start_offset > 0 {
        let previous = source.as_bytes()[start_offset - 1];
        if previous == b'\n' {
            break;
        }
        if !previous.is_ascii_whitespace() {
            break;
        }
        start_offset -= 1;
    }

    let mut trailing_offset = end.offset;
    let end_offset = loop {
        if trailing_offset >= source.len() {
            break trailing_offset;
        }
        let byte = source.as_bytes()[trailing_offset];
        if byte == b'\n' {
            break trailing_offset + 1;
        }
        if !byte.is_ascii_whitespace() {
            break end.offset;
        }
        trailing_offset += 1;
    };

    Some(Span::from_positions(
        Position {
            offset: start_offset,
            line: first.start.line,
            column: first
                .start
                .column
                .saturating_sub(first.start.offset.saturating_sub(start_offset)),
        },
        end.advanced_by(&source[end.offset..end_offset]),
    ))
}
