use shuck_ast::{Span, Word, WordPart};

use crate::rules::common::query::{self, CommandWalkOptions, visit_command_words};
use crate::{Checker, Rule, Violation};

pub struct PositionalTenBraces;

impl Violation for PositionalTenBraces {
    fn rule() -> Rule {
        Rule::PositionalTenBraces
    }

    fn message(&self) -> String {
        "use braces for positional parameters above 9".to_owned()
    }
}

pub fn positional_ten_braces(checker: &mut Checker) {
    let source = checker.source();
    let mut spans = Vec::new();

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| {
            visit_command_words(command, &mut |word| {
                collect_positional_parameter_spans(word, source, &mut spans);
            });
        },
    );

    for span in spans {
        checker.report(PositionalTenBraces, span);
    }
}

fn collect_positional_parameter_spans(word: &Word, source: &str, spans: &mut Vec<Span>) {
    collect_positional_parameter_spans_in_parts(&word.parts, source, spans);
}

fn collect_positional_parameter_spans_in_parts(
    parts: &[shuck_ast::WordPartNode],
    source: &str,
    spans: &mut Vec<Span>,
) {
    for (index, part) in parts.iter().enumerate() {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                collect_positional_parameter_spans_in_parts(parts, source, spans);
            }
            WordPart::Variable(name)
                if matches!(
                    name.as_str(),
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"
                ) =>
            {
                let Some(next_part) = parts.get(index + 1) else {
                    continue;
                };
                let WordPart::Literal(text) = &next_part.kind else {
                    continue;
                };
                if text
                    .as_str(source, next_part.span)
                    .starts_with(|char: char| char.is_ascii_digit())
                {
                    spans.push(part.span.merge(next_part.span));
                }
            }
            _ => {}
        }
    }
}
