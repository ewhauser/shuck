use shuck_ast::{Command, CompoundCommand, Pattern, PatternPart};

use crate::rules::common::query::{self, CommandWalkOptions};
use crate::rules::common::word::classify_word;
use crate::{Checker, Rule, Violation};

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

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| {
            let Command::Compound(CompoundCommand::Case(case), _) = command else {
                return;
            };

            for item in &case.cases {
                for pattern in &item.patterns {
                    if pattern_has_expansions(pattern, checker.source()) {
                        spans.push(pattern.span);
                    }
                }
            }
        },
    );

    for span in spans {
        checker.report(CasePatternVar, span);
    }
}

fn pattern_has_expansions(pattern: &Pattern, source: &str) -> bool {
    for (part, _) in pattern.parts_with_spans() {
        match part {
            PatternPart::Group { patterns, .. } => {
                if patterns
                    .iter()
                    .any(|pattern| pattern_has_expansions(pattern, source))
                {
                    return true;
                }
            }
            PatternPart::Word(word) => {
                if classify_word(word, source).is_expanded() {
                    return true;
                }
            }
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }

    false
}
