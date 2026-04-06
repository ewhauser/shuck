use shuck_ast::{Command, CompoundCommand, Word, WordPart};

use crate::rules::common::query::{self, CommandWalkOptions};
use crate::{Checker, Rule, Violation};

pub struct LineOrientedInput;

impl Violation for LineOrientedInput {
    fn rule() -> Rule {
        Rule::LineOrientedInput
    }

    fn message(&self) -> String {
        "iterating over command output in a `for` loop splits lines on whitespace".to_owned()
    }
}

pub fn line_oriented_input(checker: &mut Checker) {
    let mut spans = Vec::new();

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| {
            let Command::Compound(CompoundCommand::For(command), _) = command else {
                return;
            };

            let Some(words) = &command.words else {
                return;
            };

            for word in words {
                if word_contains_command_substitution(word) {
                    spans.push(word.span);
                }
            }
        },
    );

    for span in spans {
        checker.report(LineOrientedInput, span);
    }
}

fn word_contains_command_substitution(word: &Word) -> bool {
    word.parts
        .iter()
        .any(|part| matches!(part, WordPart::CommandSubstitution(_)))
}
