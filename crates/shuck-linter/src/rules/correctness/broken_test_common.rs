use shuck_ast::{Span, static_word_text};

use crate::Checker;

pub(super) fn malformed_bracket_test_spans(checker: &Checker<'_>) -> Vec<Span> {
    checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.static_utility_name_is("["))
        .filter(|fact| {
            fact.body_args()
                .last()
                .and_then(|word| static_word_text(word, checker.source()))
                .as_deref()
                != Some("]")
        })
        .map(|fact| fact.body_name_word().map_or(fact.span(), |word| word.span))
        .collect()
}
