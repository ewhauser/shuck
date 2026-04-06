use crate::{Checker, Rule, Violation};

use super::syntax::walk_words;

pub struct LegacyBackticks;

impl Violation for LegacyBackticks {
    fn rule() -> Rule {
        Rule::LegacyBackticks
    }

    fn message(&self) -> String {
        "prefer `$(...)` over legacy backtick substitution".to_owned()
    }
}

pub fn legacy_backticks(checker: &mut Checker) {
    let source = checker.source();
    let mut spans = Vec::new();

    walk_words(&checker.ast().commands, &mut |word| {
        if word_uses_backticks(word.span.slice(source)) {
            spans.push(word.span);
        }
    });

    spans.sort_unstable_by_key(|span| (span.start.offset, usize::MAX - span.end.offset));

    let mut filtered = Vec::new();
    for span in spans {
        if filtered.last().is_some_and(|previous: &shuck_ast::Span| {
            previous.start.offset <= span.start.offset && previous.end.offset >= span.end.offset
        }) {
            continue;
        }
        filtered.push(span);
    }

    for span in filtered {
        checker.report(LegacyBackticks, span);
    }
}

fn word_uses_backticks(text: &str) -> bool {
    let mut in_single_quotes = false;
    let mut escaped = false;

    for ch in text.chars() {
        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' if !in_single_quotes => escaped = true,
            '\'' => in_single_quotes = !in_single_quotes,
            '`' if !in_single_quotes => return true,
            _ => {}
        }
    }

    false
}
