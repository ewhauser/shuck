use shuck_ast::{
    ArithmeticExpansionSyntax, Assignment, CommandListItem, CommandSubstitutionSyntax, Position,
    Redirect, Span, TextRange, TextSize, Word, WordPart, WordPartNode,
};
use shuck_indexer::{Indexer, RegionKind};

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
    let mut spans = Vec::new();
    collect_command_substitution_spans(&word.parts, &mut spans);
    spans
}

pub fn unquoted_command_substitution_part_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_unquoted_command_substitution_spans(&word.parts, false, &mut spans);
    spans
}

pub fn array_expansion_part_spans(word: &Word, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_array_expansion_spans(&word.parts, source, false, false, &mut spans);
    spans
}

pub fn unquoted_array_expansion_part_spans(word: &Word, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_array_expansion_spans(&word.parts, source, false, true, &mut spans);
    spans
}

pub fn expansion_part_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_expansion_spans(&word.parts, &mut spans);
    spans
}

pub fn scalar_expansion_part_spans(word: &Word, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_scalar_expansion_spans(&word.parts, source, &mut spans);
    spans
}

pub fn single_quoted_region_span(indexer: &Indexer, span: Span) -> Span {
    let offset = TextSize::new(span.start.offset as u32);
    let Some(range) = indexer.region_index().single_quoted_range_at(offset) else {
        return span;
    };

    text_range_span(indexer, range)
}

pub fn backtick_fragment_spans(word: &Word, _source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_backtick_spans(&word.parts, &mut spans);
    spans
}

pub fn legacy_arithmetic_part_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_legacy_arithmetic_spans(&word.parts, &mut spans);
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

pub fn is_quoted_span(indexer: &Indexer, span: Span) -> bool {
    matches!(
        indexer
            .region_index()
            .region_at(TextSize::new(span.start.offset as u32)),
        Some(RegionKind::SingleQuoted | RegionKind::DoubleQuoted)
    )
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
    source: &str,
    quoted: bool,
    only_unquoted: bool,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_array_expansion_spans(parts, source, true, only_unquoted, spans)
            }
            WordPart::ArrayAccess { index, .. }
                if matches!(index.slice(source), "@" | "*") && (!quoted || !only_unquoted) =>
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
            WordPart::Variable(_)
            | WordPart::CommandSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
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
            | WordPart::Transformation { .. } => spans.push(part.span),
        }
    }
}

fn collect_scalar_expansion_spans(parts: &[WordPartNode], source: &str, spans: &mut Vec<Span>) {
    for part in parts {
        match &part.kind {
            WordPart::Literal(_) | WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_scalar_expansion_spans(parts, source, spans)
            }
            WordPart::CommandSubstitution { .. } | WordPart::ProcessSubstitution { .. } => {}
            WordPart::Variable(_)
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayLength(_)
            | WordPart::Substring { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::PrefixMatch(_)
            | WordPart::Transformation { .. } => spans.push(part.span),
            WordPart::ArrayAccess { index, .. } => {
                if !matches!(index.slice(source), "@" | "*") {
                    spans.push(part.span);
                }
            }
            WordPart::ArrayIndices(_) | WordPart::ArraySlice { .. } => {}
        }
    }
}

fn collect_backtick_spans(parts: &[WordPartNode], spans: &mut Vec<Span>) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => collect_backtick_spans(parts, spans),
            WordPart::CommandSubstitution {
                syntax: CommandSubstitutionSyntax::Backtick,
                ..
            } => {
                spans.push(part.span);
            }
            _ => {}
        }
    }
}

fn collect_legacy_arithmetic_spans(parts: &[WordPartNode], spans: &mut Vec<Span>) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => collect_legacy_arithmetic_spans(parts, spans),
            WordPart::ArithmeticExpansion {
                syntax: ArithmeticExpansionSyntax::LegacyBracket,
                ..
            } => {
                spans.push(part.span);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use shuck_parser::parser::Parser;

    use super::{
        array_expansion_part_spans, backtick_fragment_spans, command_substitution_part_spans,
        scalar_expansion_part_spans,
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
    fn scalar_expansion_spans_ignore_array_splats_and_command_substitutions() {
        let source = "printf '%s\\n' prefix${name}suffix ${arr[@]} ${arr[0]} $(date)\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.script.commands[0];
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
    fn backtick_fragment_spans_find_exact_pairs() {
        let source = "echo \"today is `date` and `uname`\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.script.commands[0];
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        let spans = backtick_fragment_spans(word, source);
        assert_eq!(
            spans
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["`date`", "`uname`"]
        );
    }
}
