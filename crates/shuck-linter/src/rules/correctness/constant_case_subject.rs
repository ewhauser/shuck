use shuck_ast::{Command, CompoundCommand};

use crate::rules::common::query::{self, CommandWalkOptions};
use crate::rules::common::word::classify_word;
use crate::{Checker, Rule, Violation};

pub struct ConstantCaseSubject;

impl Violation for ConstantCaseSubject {
    fn rule() -> Rule {
        Rule::ConstantCaseSubject
    }

    fn message(&self) -> String {
        "this `case` statement switches on a fixed literal".to_owned()
    }
}

pub fn constant_case_subject(checker: &mut Checker) {
    let source = checker.source();
    let mut spans = Vec::new();

    query::walk_commands(
        &checker.ast().body,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |visit| {
            let command = visit.command;
            let Command::Compound(CompoundCommand::Case(command)) = command else {
                return;
            };

            if classify_word(&command.word, source).is_fixed_literal() {
                spans.push(command.word.span);
            }
        },
    );

    for span in spans {
        checker.report(ConstantCaseSubject, span);
    }
}
