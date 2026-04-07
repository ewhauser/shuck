use shuck_ast::{ArrayElem, Assignment, AssignmentValue, Command, DeclOperand, Word};

use crate::rules::common::span;
use crate::rules::common::word::classify_word;
use crate::rules::common::{
    expansion::ExpansionContext,
    query::{self, CommandWalkOptions},
};
use crate::{Checker, Rule, Violation};

pub struct UnquotedCommandSubstitution;

impl Violation for UnquotedCommandSubstitution {
    fn rule() -> Rule {
        Rule::UnquotedCommandSubstitution
    }

    fn message(&self) -> String {
        "quote command substitutions in arguments to avoid word splitting".to_owned()
    }
}

pub fn unquoted_command_substitution(checker: &mut Checker) {
    let source = checker.source();

    query::walk_commands(
        &checker.ast().commands,
        checker.source(),
        CommandWalkOptions {
            descend_nested_word_commands: false,
        },
        &mut |command, _| {
            query::visit_expansion_words(command, source, &mut |word, context| {
                if context != ExpansionContext::CommandArgument {
                    return;
                }

                report_command_substitution_word(checker, word, source);
            });

            visit_command_subscript_words(command, source, &mut |word| {
                report_command_substitution_word(checker, word, source);
            });
        },
    );
}

fn report_command_substitution_word(checker: &mut Checker, word: &Word, source: &str) {
    let classification = classify_word(word, source);
    if classification.has_command_substitution() {
        for span in span::unquoted_command_substitution_part_spans(word) {
            checker.report_dedup(UnquotedCommandSubstitution, span);
        }
    }
}

fn visit_command_subscript_words(command: &Command, source: &str, visitor: &mut impl FnMut(&Word)) {
    for assignment in query::command_assignments(command) {
        visit_assignment_subscript_words(assignment, source, visitor);
    }

    for operand in query::declaration_operands(command) {
        match operand {
            DeclOperand::Flag(_) | DeclOperand::Dynamic(_) => {}
            DeclOperand::Name(reference) => {
                query::visit_var_ref_subscript_words(reference, source, visitor);
            }
            DeclOperand::Assignment(assignment) => {
                visit_assignment_subscript_words(assignment, source, visitor);
            }
        }
    }
}

fn visit_assignment_subscript_words(
    assignment: &Assignment,
    source: &str,
    visitor: &mut impl FnMut(&Word),
) {
    query::visit_var_ref_subscript_words(&assignment.target, source, visitor);

    let AssignmentValue::Compound(array) = &assignment.value else {
        return;
    };

    for element in &array.elements {
        match element {
            ArrayElem::Sequential(_) => {}
            ArrayElem::Keyed { key, .. } | ArrayElem::KeyedAppend { key, .. } => {
                query::visit_subscript_words(Some(key), source, visitor);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_inner_command_substitution_spans() {
        let source = "printf '%s\\n' prefix$(date)suffix $(uname)\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$(date)", "$(uname)"]
        );
    }

    #[test]
    fn ignores_redirect_and_here_string_contexts() {
        let source = "\
#!/bin/bash
cat <<< $(printf here) >$(printf out)
printf '%s\\n' $(printf arg)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$(printf arg)"]
        );
    }

    #[test]
    fn reports_subscript_command_substitutions_without_flagging_assignment_rhs() {
        let source = "\
declare arr[$(printf hi)]=1
stamp=$(printf ok)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$(printf hi)"]
        );
    }
}
