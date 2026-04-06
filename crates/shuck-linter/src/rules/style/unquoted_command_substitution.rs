use shuck_ast::Word;

use crate::rules::common::query::{self, CommandWalkOptions};
use crate::{Checker, Rule, Violation};

use super::syntax::{visit_argument_words, word_contains_command_substitution};

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
                if word_has_unquoted_command_substitution(word) {
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

fn word_has_unquoted_command_substitution(word: &Word) -> bool {
    !word.quoted && word_contains_command_substitution(word)
}
