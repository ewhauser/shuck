use shuck_ast::{Command, CompoundCommand, Pipeline, Word, WordPart};

use crate::rules::common::{
    command,
    query::{self, CommandWalkOptions},
};
use crate::{Checker, Rule, Violation};

pub struct FindOutputLoop;

impl Violation for FindOutputLoop {
    fn rule() -> Rule {
        Rule::FindOutputLoop
    }

    fn message(&self) -> String {
        "expanding `find` output in a `for` loop splits paths on whitespace".to_owned()
    }
}

pub fn find_output_loop(checker: &mut Checker) {
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
                if word_contains_find_substitution(word, source) {
                    spans.push(word.span);
                }
            }
        },
    );

    for span in spans {
        checker.report(FindOutputLoop, span);
    }
}

fn word_contains_find_substitution(word: &Word, source: &str) -> bool {
    word.parts.iter().any(|part| match part {
        WordPart::CommandSubstitution(commands)
        | WordPart::ProcessSubstitution { commands, .. } => {
            commands_start_with_find(commands, source)
        }
        _ => false,
    })
}

fn commands_start_with_find(commands: &[Command], source: &str) -> bool {
    matches!(commands, [command] if command_starts_with_find(command, source))
}

fn command_starts_with_find(command: &Command, source: &str) -> bool {
    match command {
        Command::Pipeline(Pipeline { commands, .. }) => {
            matches!(commands.as_slice(), [command] if command_starts_with_find(command, source))
        }
        Command::List(command) => {
            command.rest.is_empty() && command_starts_with_find(&command.first, source)
        }
        _ => command::normalize_command(command, source).effective_name_is("find"),
    }
}
