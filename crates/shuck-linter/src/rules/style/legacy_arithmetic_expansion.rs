use crate::rules::common::{
    query::{self, CommandWalkOptions},
    span,
};
use crate::{Checker, Rule, Violation};

pub struct LegacyArithmeticExpansion;

impl Violation for LegacyArithmeticExpansion {
    fn rule() -> Rule {
        Rule::LegacyArithmeticExpansion
    }

    fn message(&self) -> String {
        "prefer `$((...))` over legacy `$[...]` arithmetic expansion".to_owned()
    }
}

pub fn legacy_arithmetic_expansion(checker: &mut Checker) {
    let mut spans = Vec::new();

    query::walk_words(
        &checker.ast().body,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |word| {
            spans.extend(span::legacy_arithmetic_part_spans(word));
        },
    );

    spans.sort_unstable_by_key(|span| (span.start.offset, span.end.offset));
    spans.dedup();

    for span in spans {
        checker.report(LegacyArithmeticExpansion, span);
    }
}
