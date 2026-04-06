use shuck_ast::Command;

use crate::rules::common::query::{self, CommandWalkOptions};
use crate::{Checker, Rule, Violation};

use super::syntax::static_word_text;

pub struct SudoRedirectionOrder;

impl Violation for SudoRedirectionOrder {
    fn rule() -> Rule {
        Rule::SudoRedirectionOrder
    }

    fn message(&self) -> String {
        "redirections on `sudo` still run in the current shell".to_owned()
    }
}

pub fn sudo_redirection_order(checker: &mut Checker) {
    let source = checker.source();
    let mut spans = Vec::new();

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| {
            let Command::Simple(command) = command else {
                return;
            };

            if !command.redirects.is_empty()
                && static_word_text(&command.name, source).as_deref() == Some("sudo")
            {
                spans.push(command.span);
            }
        },
    );

    for span in spans {
        checker.report(SudoRedirectionOrder, span);
    }
}
