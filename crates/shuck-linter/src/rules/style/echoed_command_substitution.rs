use crate::rules::common::{
    command,
    query::{self, CommandWalkOptions},
};
use crate::{Checker, Rule, Violation};

use super::syntax::word_is_plain_command_substitution;

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

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| {
            let normalized = command::normalize_command(command, source);
            if !normalized.effective_name_is("echo") {
                return;
            }

            let [word] = normalized.body_args() else {
                return;
            };

            if word_is_plain_command_substitution(word) {
                spans.push(normalized.body_span);
            }
        },
    );

    for span in spans {
        checker.report(EchoedCommandSubstitution, span);
    }
}
