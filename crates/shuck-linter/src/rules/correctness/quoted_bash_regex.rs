use shuck_ast::{Command, CompoundCommand, ConditionalBinaryOp, ConditionalExpr};

use crate::{Checker, Rule, Violation};

use super::syntax::walk_commands;

pub struct QuotedBashRegex;

impl Violation for QuotedBashRegex {
    fn rule() -> Rule {
        Rule::QuotedBashRegex
    }

    fn message(&self) -> String {
        "quoting the right-hand side of `=~` forces a literal string match".to_owned()
    }
}

pub fn quoted_bash_regex(checker: &mut Checker) {
    let mut spans = Vec::new();

    walk_commands(&checker.ast().commands, &mut |command, _| {
        let Command::Compound(CompoundCommand::Conditional(command), _) = command else {
            return;
        };

        let ConditionalExpr::Binary(expression) = &command.expression else {
            return;
        };

        if expression.op != ConditionalBinaryOp::RegexMatch {
            return;
        }

        let ConditionalExpr::Regex(word) = expression.right.as_ref() else {
            return;
        };

        if word.quoted {
            spans.push(word.span);
        }
    });

    for span in spans {
        checker.report(QuotedBashRegex, span);
    }
}
