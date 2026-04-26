use rustc_hash::FxHashSet;
use shuck_ast::{
    ArenaFileCommandKind, AssignmentNode, AssignmentValueNode, Span, static_word_text_arena,
};
use shuck_semantic::{Binding, BindingAttributes, BindingKind};

use crate::{Checker, CommandFactRef, ExpansionContext, Rule, Violation, WordFactContext, WordQuote};

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
        .filter(|binding| binding_contributes_known_variable_name(binding))
        .map(|binding| binding.name.as_str().to_owned())
        .collect::<FxHashSet<_>>();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.literal_name() == Some(""))
        .flat_map(|fact| command_assignment_spans(checker, fact, source, &known_names))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || AssignmentLooksLikeComparison);
}

fn command_assignment_spans(
    checker: &Checker<'_>,
    command: CommandFactRef<'_, '_>,
    source: &str,
    known_names: &FxHashSet<String>,
) -> Vec<Span> {
    match command.command_kind() {
        ArenaFileCommandKind::Simple => command
            .arena_assignments()
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
        ArenaFileCommandKind::Builtin
        | ArenaFileCommandKind::Decl
        | ArenaFileCommandKind::Binary
        | ArenaFileCommandKind::Compound
        | ArenaFileCommandKind::Function
        | ArenaFileCommandKind::AnonymousFunction => Vec::new(),
    }
}

fn assignment_value_looks_like_comparison(
    checker: &Checker<'_>,
    assignment: &AssignmentNode,
    source: &str,
    known_names: &FxHashSet<String>,
    context: WordFactContext,
) -> Option<Span> {
    let AssignmentValueNode::Scalar(word_id) = assignment.value else {
        return None;
    };
    let word = checker.facts().arena_file().store.word(word_id);

    let fact = checker.facts().word_fact(word.span(), context)?;
    if fact.classification().quote != WordQuote::Unquoted {
        return None;
    }

    let target = assignment.target.name.as_str();
    let value = static_word_text_arena(word, source)?;
    let (prefix, remainder) = value.split_once('-')?;
    if remainder.is_empty() {
        return None;
    }

    if prefix == target || known_names.contains(prefix) {
        Some(word.span())
    } else {
        None
    }
}

fn binding_contributes_known_variable_name(binding: &Binding) -> bool {
    match binding.kind {
        BindingKind::FunctionDefinition => false,
        BindingKind::Imported => !binding
            .attributes
            .contains(BindingAttributes::IMPORTED_FUNCTION),
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
        | BindingKind::ArithmeticAssignment
        | BindingKind::Nameref => true,
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
}
