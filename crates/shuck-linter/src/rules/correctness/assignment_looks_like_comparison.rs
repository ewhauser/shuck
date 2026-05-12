use rustc_hash::FxHashSet;
use shuck_ast::{Assignment, AssignmentValue, Command, Span, static_word_text};
use shuck_semantic::{Binding, BindingKind};

use crate::{
    Checker, Diagnostic, Edit, ExpansionContext, Fix, FixAvailability, Rule, Violation,
    WordFactContext, WordQuote,
};

pub struct AssignmentLooksLikeComparison;

impl Violation for AssignmentLooksLikeComparison {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::AssignmentLooksLikeComparison
    }

    fn message(&self) -> String {
        "assignment value looks like arithmetic subtraction".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("quote the assignment value".to_owned())
    }
}

pub fn assignment_looks_like_comparison(checker: &mut Checker) {
    let source = checker.source();
    let known_names = checker
        .semantic()
        .bindings()
        .iter()
        .filter(|binding| binding_contributes_known_variable_name(binding))
        .map(|binding| binding.name.as_str().to_owned())
        .collect::<FxHashSet<_>>();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.literal_name() == Some(""))
        .flat_map(|fact| command_assignment_spans(checker, fact.command(), source, &known_names))
        .collect::<Vec<_>>();

    for span in spans {
        checker.report_diagnostic_dedup(
            Diagnostic::new(AssignmentLooksLikeComparison, span).with_fix(Fix::safe_edit(
                Edit::replacement(format!("\"{}\"", span.slice(source)), span),
            )),
        );
    }
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
        Command::Builtin(_)
        | Command::Decl(_)
        | Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => Vec::new(),
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

    if prefix == target || known_names.contains(prefix) {
        Some(word.span)
    } else {
        None
    }
}

fn binding_contributes_known_variable_name(binding: &Binding) -> bool {
    match binding.kind {
        BindingKind::FunctionDefinition => false,
        // Imported and ambient bindings are too broad for this heuristic:
        // they make ordinary hyphenated literals like `history-expansion`
        // look like subtraction just because `history` exists at runtime.
        BindingKind::Imported => false,
        BindingKind::Assignment
        | BindingKind::ParameterDefaultAssignment
        | BindingKind::AppendAssignment
        | BindingKind::ArrayAssignment
        | BindingKind::Declaration(_)
        | BindingKind::LoopVariable
        | BindingKind::ReadTarget
        | BindingKind::MapfileTarget
        | BindingKind::PrintfTarget
        | BindingKind::GetoptsTarget
        | BindingKind::ZparseoptsTarget
        | BindingKind::ArithmeticAssignment
        | BindingKind::Nameref => true,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{
        test_path_with_fix, test_snippet, test_snippet_at_path, test_snippet_with_fix,
    };
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

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
FOO=lower-baz
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

    #[test]
    fn ignores_command_environment_and_declaration_assignments() {
        let source = "\
#!/bin/bash
foo=foo-1 env
foo=foo-2 echo hi
export foo=foo-3
readonly foo=foo-4
local foo=foo-5
declare foo=foo-6
typeset foo=foo-7
foo=foo-8
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
            vec!["foo-8"]
        );
    }

    #[test]
    fn ignores_function_names_as_prefix_candidates() {
        let source = "\
#!/bin/bash
en() { :; }
lang=en-us
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AssignmentLooksLikeComparison),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_ambient_runtime_names_as_prefix_candidates() {
        let path =
            Path::new("/tmp/zsh/zsh-syntax-highlighting/highlighters/main/main-highlighter.zsh");
        let source = "\
print -r -- \"$history\"
style=history-expansion
";
        let diagnostics = test_snippet_at_path(
            path,
            source,
            &LinterSettings::for_rule(Rule::AssignmentLooksLikeComparison),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn applies_safe_fix_to_literal_assignment_values() {
        let source = "\
#!/bin/bash
foo=foo-bar
foo+=foo-1
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::AssignmentLooksLikeComparison),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
foo=\"foo-bar\"
foo+=\"foo-1\"
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_non_matching_values_unchanged_when_fixing() {
        let source = "\
#!/bin/bash
foo=bar-baz
foo=\"$foo-bar\"
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::AssignmentLooksLikeComparison),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_safe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C095.sh").as_path(),
            &LinterSettings::for_rule(Rule::AssignmentLooksLikeComparison),
            Applicability::Safe,
        )?;

        assert_diagnostics_diff!("C095_fix_C095.sh", result);
        Ok(())
    }
}
