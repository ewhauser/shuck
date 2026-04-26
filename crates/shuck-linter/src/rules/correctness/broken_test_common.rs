use shuck_ast::Span;

use crate::Checker;

pub(super) fn malformed_bracket_test_spans(checker: &Checker<'_>) -> Vec<Span> {
    checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.static_utility_name_is("["))
        .filter(|fact| {
            fact.arena_body_args(checker.source())
                .last()
                .and_then(|word| word.static_text(checker.source()))
                .as_deref()
                != Some("]")
        })
        .map(|fact| {
            fact.arena_body_name_word(checker.source())
                .map_or(fact.span(), |word| word.span())
        })
        .collect()
}
