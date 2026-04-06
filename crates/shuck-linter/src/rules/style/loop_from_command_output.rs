use shuck_ast::{Command, CompoundCommand, Word};

use crate::rules::common::query::{self, CommandWalkOptions};
use crate::{Checker, Rule, Violation};

use super::syntax::word_contains_command_substitution;

pub struct LoopFromCommandOutput;

impl Violation for LoopFromCommandOutput {
    fn rule() -> Rule {
        Rule::LoopFromCommandOutput
    }

    fn message(&self) -> String {
        "iterating over command output is fragile; use globs, arrays, or explicit delimiters"
            .to_owned()
    }
}

pub fn loop_from_command_output(checker: &mut Checker) {
    let mut spans = Vec::new();

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: false,
        },
        &mut |command, _| {
            let Command::Compound(CompoundCommand::For(command), _) = command else {
                return;
            };

            let Some(words) = &command.words else {
                return;
            };

            for word in words {
                if word_contains_unquoted_command_output(word) {
                    spans.push(word.span);
                }
            }
        },
    );

    for span in spans {
        checker.report(LoopFromCommandOutput, span);
    }
}

fn word_contains_unquoted_command_output(word: &Word) -> bool {
    !word.quoted && word_contains_command_substitution(word)
}
