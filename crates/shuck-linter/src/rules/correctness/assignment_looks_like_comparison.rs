use rustc_hash::FxHashSet;
use shuck_ast::{Assignment, AssignmentValue, BuiltinCommand, Command, DeclOperand, Span};

use crate::{
    Checker, ExpansionContext, Rule, Violation, WordFactContext, WordQuote, static_word_text,
};

pub struct AssignmentLooksLikeComparison;

impl Violation for AssignmentLooksLikeComparison {
    fn rule() -> Rule {
        Rule::AssignmentLooksLikeComparison
    }

    fn message(&self) -> String {
        "assignment value looks like arithmetic subtraction".to_owned()
    }
}

pub fn assignment_looks_like_comparison(checker: &mut Checker) {
    let source = checker.source();
    let known_names = checker
        .semantic()
        .bindings()
        .iter()
        .map(|binding| binding.name.as_str().to_owned())
        .collect::<FxHashSet<_>>();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| command_assignment_spans(checker, fact.command(), source, &known_names))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || AssignmentLooksLikeComparison);
}

fn command_assignment_spans(
    checker: &Checker<'_>,
    command: &Command,
    source: &str,
    known_names: &FxHashSet<String>,
) -> Vec<Span> {
    match command {
        Command::Simple(command) => command
            .assignments
            .iter()
            .filter_map(|assignment| {
                assignment_value_looks_like_comparison(
                    checker,
                    assignment,
                    source,
                    known_names,
                    WordFactContext::Expansion(ExpansionContext::AssignmentValue),
                )
            })
            .collect(),
        Command::Builtin(command) => builtin_assignments(command)
            .iter()
            .filter_map(|assignment| {
                assignment_value_looks_like_comparison(
                    checker,
                    assignment,
                    source,
                    known_names,
                    WordFactContext::Expansion(ExpansionContext::AssignmentValue),
                )
            })
            .collect(),
        Command::Decl(command) => command
            .assignments
            .iter()
            .chain(command.operands.iter().filter_map(|operand| match operand {
                DeclOperand::Assignment(assignment) => Some(assignment),
                DeclOperand::Flag(_) | DeclOperand::Name(_) | DeclOperand::Dynamic(_) => None,
            }))
            .filter_map(|assignment| {
                assignment_value_looks_like_comparison(
                    checker,
                    assignment,
                    source,
                    known_names,
                    WordFactContext::Expansion(ExpansionContext::DeclarationAssignmentValue),
                )
            })
            .collect(),
        Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => Vec::new(),
    }
}

fn builtin_assignments(command: &BuiltinCommand) -> &[Assignment] {
    match command {
        BuiltinCommand::Break(command) => &command.assignments,
        BuiltinCommand::Continue(command) => &command.assignments,
        BuiltinCommand::Return(command) => &command.assignments,
        BuiltinCommand::Exit(command) => &command.assignments,
    }
}

fn assignment_value_looks_like_comparison(
    checker: &Checker<'_>,
    assignment: &Assignment,
    source: &str,
    known_names: &FxHashSet<String>,
    context: WordFactContext,
) -> Option<Span> {
    let AssignmentValue::Scalar(word) = &assignment.value else {
        return None;
    };

    let fact = checker.facts().word_fact(word.span, context)?;
    if fact.classification().quote != WordQuote::Unquoted {
        return None;
    }

    let target = assignment.target.name.as_str();
    let value = static_word_text(word, source)?;
    let (prefix, remainder) = value.split_once('-')?;
    if remainder.is_empty() {
        return None;
    }

    if prefix.eq_ignore_ascii_case(target) || known_names.contains(prefix) {
        Some(word.span)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_assignment_values() {
        let source = "\
#!/bin/bash
foo=foo-bar
foo+=foo-1
bar=bar_baz
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AssignmentLooksLikeComparison),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["foo-bar", "foo-1"]
        );
    }

    #[test]
    fn ignores_non_matching_or_non_static_assignments() {
        let source = "\
#!/bin/bash
foo=bar-baz
foo=\"$foo-bar\"
foo=${foo}-bar
foo=(foo-bar)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AssignmentLooksLikeComparison),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_values_that_start_with_another_known_name() {
        let source = "\
#!/bin/bash
schedule=1
BASE_IMAGE_JOB_TOPIC=schedule-base-image-build
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AssignmentLooksLikeComparison),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["schedule-base-image-build"]
        );
    }
}
