use shuck_ast::Command;

use crate::{Checker, Rule, Violation};

use super::syntax::{static_word_text, walk_commands, word_is_plain_command_substitution};

pub struct EchoedCommandSubstitution;

impl Violation for EchoedCommandSubstitution {
    fn rule() -> Rule {
        Rule::EchoedCommandSubstitution
    }

    fn message(&self) -> String {
        "call the command directly instead of echoing its substitution".to_owned()
    }
}

pub fn echoed_command_substitution(checker: &mut Checker) {
    let source = checker.source();
    let mut spans = Vec::new();

    walk_commands(&checker.ast().commands, &mut |command| {
        let Command::Simple(command) = command else {
            return;
        };

        if static_word_text(&command.name, source).as_deref() != Some("echo") {
            return;
        }

        let [word] = command.args.as_slice() else {
            return;
        };

        if word_is_plain_command_substitution(word) {
            spans.push(command.span);
        }
    });

    for span in spans {
        checker.report(EchoedCommandSubstitution, span);
    }
}
