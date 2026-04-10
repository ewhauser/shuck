use shuck_ast::Span;

use crate::{Checker, static_word_text};

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
