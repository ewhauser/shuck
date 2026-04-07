use shuck_ast::{ParameterOp, Pattern, PatternPart, WordPart};

use crate::rules::common::query::{self, CommandWalkOptions, visit_command_words};
use crate::rules::common::word::classify_word;
use crate::{Checker, Rule, Violation};

pub struct PatternWithVariable;

impl Violation for PatternWithVariable {
    fn rule() -> Rule {
        Rule::PatternWithVariable
    }

    fn message(&self) -> String {
        "pattern expressions should not expand variables".to_owned()
    }
}

pub fn pattern_with_variable(checker: &mut Checker) {
    let source = checker.source();
    let mut spans = Vec::new();

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| {
            visit_command_words(command, &mut |word| {
                for (part, span) in word.parts_with_spans() {
                    let WordPart::ParameterExpansion { operator, .. } = part else {
                        continue;
                    };

                    if pattern_uses_variable(operator, source) {
                        spans.push(span);
                    }
                }
            });
        },
    );

    for span in spans {
        checker.report(PatternWithVariable, span);
    }
}

fn pattern_uses_variable(operator: &ParameterOp, source: &str) -> bool {
    match operator {
        ParameterOp::RemovePrefixShort { pattern }
        | ParameterOp::RemovePrefixLong { pattern }
        | ParameterOp::RemoveSuffixShort { pattern }
        | ParameterOp::RemoveSuffixLong { pattern } => pattern_has_dynamic_fragment(pattern, source),
        ParameterOp::ReplaceFirst { pattern, .. } | ParameterOp::ReplaceAll { pattern, .. } => {
            pattern_has_dynamic_fragment(pattern, source)
        }
        ParameterOp::UseDefault
        | ParameterOp::AssignDefault
        | ParameterOp::UseReplacement
        | ParameterOp::Error
        | ParameterOp::UpperFirst
        | ParameterOp::UpperAll
        | ParameterOp::LowerFirst
        | ParameterOp::LowerAll => false,
    }
}

fn pattern_has_dynamic_fragment(pattern: &Pattern, source: &str) -> bool {
    for (part, _) in pattern.parts_with_spans() {
        match part {
            PatternPart::Group { patterns, .. } => {
                if patterns
                    .iter()
                    .any(|pattern| pattern_has_dynamic_fragment(pattern, source))
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
