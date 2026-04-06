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
    for (index, (part, span)) in word.parts_with_spans().enumerate() {
        let WordPart::Variable(name) = part else {
            continue;
        };

        if !matches!(
            name.as_str(),
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"
        ) {
            continue;
        }

        let Some(WordPart::Literal(text)) = word.parts.get(index + 1) else {
            continue;
        };
        let Some(next_span) = word.part_span(index + 1) else {
            continue;
        };

        if text
            .as_str(source, next_span)
            .starts_with(|char: char| char.is_ascii_digit())
        {
            spans.push(span.merge(next_span));
        }
    }
}
