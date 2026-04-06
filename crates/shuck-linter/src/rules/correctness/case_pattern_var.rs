use shuck_ast::{Command, CompoundCommand};

use crate::{Checker, Rule, Violation};

use super::syntax::{walk_commands, word_has_expansion};

pub struct CasePatternVar;

impl Violation for CasePatternVar {
    fn rule() -> Rule {
        Rule::CasePatternVar
    }

    fn message(&self) -> String {
        "case patterns should be literal instead of built from expansions".to_owned()
    }
}

pub fn case_pattern_var(checker: &mut Checker) {
    let mut spans = Vec::new();

    walk_commands(&checker.ast().commands, &mut |command, _| {
        let Command::Compound(CompoundCommand::Case(case), _) = command else {
            return;
        };

        for item in &case.cases {
            for pattern in &item.patterns {
                if word_has_expansion(pattern) {
                    spans.push(pattern.span);
                }
            }
        }
    });

    for span in spans {
        checker.report(CasePatternVar, span);
    }
}
