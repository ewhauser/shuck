use crate::rules::common::query::{self, CommandWalkOptions};
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
    let source = checker.source();
    let mut spans = Vec::new();

    query::walk_words(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |word| {
            if word_uses_legacy_arithmetic(word.span.slice(source)) {
                spans.push(word.span);
            }
        },
    );

    spans.sort_unstable_by_key(|span| (span.start.offset, span.end.offset));
    spans.dedup();

    for span in spans {
        checker.report(LegacyArithmeticExpansion, span);
    }
}

fn word_uses_legacy_arithmetic(text: &str) -> bool {
    let mut in_single_quotes = false;
    let mut escaped = false;
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' if !in_single_quotes => escaped = true,
            '\'' => in_single_quotes = !in_single_quotes,
            '$' if !in_single_quotes && chars.peek() == Some(&'[') => return true,
            _ => {}
        }
    }

    false
}
