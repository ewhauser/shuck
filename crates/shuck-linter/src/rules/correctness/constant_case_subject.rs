use shuck_ast::{Command, CompoundCommand};

use crate::{Checker, Rule, Violation};

use super::syntax::{static_word_text, walk_commands};

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

    walk_commands(&checker.ast().commands, &mut |command, _| {
        let Command::Compound(CompoundCommand::Case(command), _) = command else {
            return;
        };

        if static_word_text(&command.word, source).is_some() {
            spans.push(command.word.span);
        }
    });

    for span in spans {
        checker.report(ConstantCaseSubject, span);
    }
}
