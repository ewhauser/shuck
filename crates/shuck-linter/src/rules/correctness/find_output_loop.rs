use shuck_ast::{BinaryOp, Command, CompoundCommand, StmtSeq, Word, WordPart};

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
        &checker.ast().body,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |visit| {
            let command = visit.command;
            let Command::Compound(CompoundCommand::For(command)) = command else {
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
    word.parts
        .iter()
        .any(|part| part_contains_find_substitution(&part.kind, source))
}

fn part_contains_find_substitution(part: &WordPart, source: &str) -> bool {
    match part {
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter()
            .any(|part| part_contains_find_substitution(&part.kind, source)),
        WordPart::CommandSubstitution { body, .. }
        | WordPart::ProcessSubstitution { body, .. } => {
            commands_start_with_find(body, source)
        }
        _ => false,
    }
}

fn commands_start_with_find(commands: &StmtSeq, source: &str) -> bool {
    matches!(commands.as_slice(), [command] if command_starts_with_find(&command.command, source))
}

fn command_starts_with_find(command: &Command, source: &str) -> bool {
    match command {
        Command::Binary(command) if matches!(command.op, BinaryOp::Pipe | BinaryOp::PipeAll) => {
            let mut commands = Vec::new();
            collect_pipeline_segments(command, &mut commands);
            matches!(commands.as_slice(), [command] if command_starts_with_find(&command.command, source))
        }
        _ => command::normalize_command(command, source).effective_name_is("find"),
    }
}

fn collect_pipeline_segments<'a>(command: &'a shuck_ast::BinaryCommand, commands: &mut Vec<&'a shuck_ast::Stmt>) {
    match &command.left.command {
        Command::Binary(left) if matches!(left.op, BinaryOp::Pipe | BinaryOp::PipeAll) => {
            collect_pipeline_segments(left, commands);
        }
        _ => commands.push(&command.left),
    }

    match &command.right.command {
        Command::Binary(right) if matches!(right.op, BinaryOp::Pipe | BinaryOp::PipeAll) => {
            collect_pipeline_segments(right, commands);
        }
        _ => commands.push(&command.right),
    }
}
