use shuck_ast::{Command, CompoundCommand};

use crate::rules::common::query::{self, CommandWalkOptions};
use crate::rules::common::word::classify_word;
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
    let source = checker.source();
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
                if classify_word(word, source).has_command_substitution() {
                    spans.push(word.span);
                }
            }
        },
    );

    for span in spans {
        checker.report(LineOrientedInput, span);
    }
}
