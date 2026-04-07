use shuck_ast::{BuiltinCommand, Command};

use crate::rules::common::query::{self, CommandWalkOptions};
use crate::rules::common::word::static_word_text;
use crate::{Checker, Rule, Violation};

pub struct InvalidExitStatus;

impl Violation for InvalidExitStatus {
    fn rule() -> Rule {
        Rule::InvalidExitStatus
    }

    fn message(&self) -> String {
        "`exit` expects a numeric status".to_owned()
    }
}

pub fn invalid_exit_status(checker: &mut Checker) {
    let source = checker.source();
    let mut spans = Vec::new();

    query::walk_commands(
        &checker.ast().body,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |visit| {
            let command = visit.command;
            let Command::Builtin(BuiltinCommand::Exit(exit)) = command else {
                return;
            };
            let Some(code) = &exit.code else {
                return;
            };
            let Some(text) = static_word_text(code, source) else {
                return;
            };

            if !text.chars().all(|char| char.is_ascii_digit()) {
                spans.push(code.span);
            }
        },
    );

    for span in spans {
        checker.report(InvalidExitStatus, span);
    }
}
