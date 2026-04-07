use shuck_ast::WordPart;

use crate::rules::common::query::{self, CommandWalkOptions, visit_command_redirects};
use crate::{Checker, Rule, Violation};

pub struct ArithmeticRedirectionTarget;

impl Violation for ArithmeticRedirectionTarget {
    fn rule() -> Rule {
        Rule::ArithmeticRedirectionTarget
    }

    fn message(&self) -> String {
        "redirection targets should not use arithmetic expansion".to_owned()
    }
}

pub fn arithmetic_redirection_target(checker: &mut Checker) {
    let mut spans = Vec::new();

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| {
            visit_command_redirects(command, &mut |redirect| {
                if redirect
                    .target
                    .parts
                    .iter()
                    .any(|part| contains_arithmetic_expansion(&part.kind))
                {
                    spans.push(redirect.target.span);
                }
            });
        },
    );

    for span in spans {
        checker.report(ArithmeticRedirectionTarget, span);
    }
}

fn contains_arithmetic_expansion(part: &WordPart) -> bool {
    match part {
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter()
            .any(|part| contains_arithmetic_expansion(&part.kind)),
        WordPart::ArithmeticExpansion { .. } => true,
        _ => false,
    }
}
