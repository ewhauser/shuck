use shuck_ast::{ParameterOp, SourceText, WordPart};

use crate::{Checker, Rule, Violation};

use super::syntax::{visit_command_words, walk_commands};

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

    walk_commands(&checker.ast().commands, &mut |command, _| {
        visit_command_words(command, &mut |word| {
            for (part, span) in word.parts_with_spans() {
                let WordPart::ParameterExpansion {
                    operator, operand, ..
                } = part
                else {
                    continue;
                };

                if pattern_uses_variable(operator, operand.as_ref(), source) {
                    spans.push(span);
                }
            }
        });
    });

    for span in spans {
        checker.report(PatternWithVariable, span);
    }
}

fn pattern_uses_variable(
    operator: &ParameterOp,
    operand: Option<&SourceText>,
    source: &str,
) -> bool {
    match operator {
        ParameterOp::RemovePrefixShort
        | ParameterOp::RemovePrefixLong
        | ParameterOp::RemoveSuffixShort
        | ParameterOp::RemoveSuffixLong => {
            operand.is_some_and(|operand| source_text_has_variable(operand, source))
        }
        ParameterOp::ReplaceFirst { pattern, .. } | ParameterOp::ReplaceAll { pattern, .. } => {
            source_text_has_variable(pattern, source)
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

fn source_text_has_variable(text: &SourceText, source: &str) -> bool {
    let text = text.slice(source);
    let bytes = text.as_bytes();

    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'$' {
            continue;
        }

        let mut backslashes = 0;
        let mut cursor = index;
        while cursor > 0 {
            cursor -= 1;
            if bytes[cursor] != b'\\' {
                break;
            }
            backslashes += 1;
        }

        if backslashes % 2 == 0 {
            return true;
        }
    }

    false
}
