use shuck_ast::{Command, CompoundCommand, Word};

use crate::{Checker, Rule, Violation};

use super::syntax::{walk_commands, word_contains_command_substitution};

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

    walk_commands(&checker.ast().commands, &mut |command| {
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
    });

    for span in spans {
        checker.report(LoopFromCommandOutput, span);
    }
}

fn word_contains_unquoted_command_output(word: &Word) -> bool {
    !word.quoted && word_contains_command_substitution(word)
}
