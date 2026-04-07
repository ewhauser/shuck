use shuck_ast::{Command, CompoundCommand};

use crate::rules::common::query::{self, CommandWalkOptions};
use crate::rules::common::word::classify_word;
use crate::{Checker, Rule, Violation};

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
                let classification = classify_word(word);
                if classification.has_command_substitution()
                    && !crate::rules::common::span::unquoted_command_substitution_part_spans(word)
                        .is_empty()
                {
                    spans.push(word.span);
                }
            }
        },
    );

    for span in spans {
        checker.report(LoopFromCommandOutput, span);
    }
}
