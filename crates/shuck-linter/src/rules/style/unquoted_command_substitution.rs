use crate::rules::common::query::{self, CommandWalkOptions};
use crate::rules::common::word::classify_word;
use crate::{Checker, Rule, Violation};

use super::syntax::visit_argument_words;

pub struct UnquotedCommandSubstitution;

impl Violation for UnquotedCommandSubstitution {
    fn rule() -> Rule {
        Rule::UnquotedCommandSubstitution
    }

    fn message(&self) -> String {
        "quote command substitutions in arguments to avoid word splitting".to_owned()
    }
}

pub fn unquoted_command_substitution(checker: &mut Checker) {
    let mut spans = Vec::new();

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: false,
        },
        &mut |command, _| {
            visit_argument_words(command, |word| {
                let classification = classify_word(word, checker.source());
                if !word.quoted && classification.has_command_substitution() {
                    spans.push(word.span);
                }
            });
        },
    );

    spans.sort_unstable_by_key(|span| (span.start.offset, span.end.offset));
    spans.dedup();

    for span in spans {
        checker.report(UnquotedCommandSubstitution, span);
    }
}
