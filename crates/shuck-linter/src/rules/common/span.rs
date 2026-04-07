use shuck_ast::{
    ArithmeticExpansionSyntax, Assignment, BinaryCommand, CommandSubstitutionSyntax, Redirect,
    Span, Word, WordPart, WordPartNode,
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

pub fn backtick_fragment_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_backtick_spans(&word.parts, &mut spans);
    spans
}

pub fn legacy_arithmetic_part_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_legacy_arithmetic_spans(&word.parts, &mut spans);
    spans
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
            WordPart::ArrayAccess(reference)
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
            WordPart::CommandSubstitution { .. } | WordPart::ProcessSubstitution { .. } => {}
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
    fn backtick_fragment_spans_find_exact_pairs() {
        let source = "echo \"today is `date` and `uname`\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        let spans = backtick_fragment_spans(word);
        assert_eq!(
            spans
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["`date`", "`uname`"]
        );
    }
}
