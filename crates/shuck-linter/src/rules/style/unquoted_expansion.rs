use crate::rules::common::query::{self, CommandWalkOptions};
use crate::rules::common::word::classify_word;
use crate::{Checker, Rule, Violation};

use super::syntax::visit_argument_words;

pub struct UnquotedExpansion;

impl Violation for UnquotedExpansion {
    fn rule() -> Rule {
        Rule::UnquotedExpansion
    }

    fn message(&self) -> String {
        "quote parameter expansions in arguments to avoid word splitting and globbing".to_owned()
    }
}

pub fn unquoted_expansion(checker: &mut Checker) {
    let source = checker.source();
    let mut spans = Vec::new();

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: false,
        },
        &mut |command, _| {
            visit_argument_words(command, |word| {
                let classification = classify_word(word, source);
                if !word.quoted && classification.has_scalar_expansion() {
                    spans.push(word.span);
                }
            });
        },
    );

    spans.sort_unstable_by_key(|span| (span.start.offset, span.end.offset));
    spans.dedup();

    for span in spans {
        checker.report(UnquotedExpansion, span);
    }
}
