use rustc_hash::FxHashSet;
use shuck_ast::{Name, Span};
use shuck_semantic::{BindingId, ScopeId, ScopeKind};

use crate::{Checker, Rule, Violation};

pub struct FunctionReferencesUnsetParam {
    pub name: String,
}

impl Violation for FunctionReferencesUnsetParam {
    fn rule() -> Rule {
        Rule::FunctionReferencesUnsetParam
    }

    fn message(&self) -> String {
        format!("function `{}` is called with too few arguments", self.name)
    }
}

pub fn function_references_unset_param(checker: &mut Checker) {
    let mut reported = FxHashSet::<BindingId>::default();
    let mut violations = Vec::<(Span, String)>::new();

    for header in checker.facts().function_headers() {
        let function = header.function();
        let Some((name, name_span)) = function.header.entries.first().and_then(|entry| {
            entry
                .static_name
                .as_ref()
                .map(|name| (name, entry.word.span))
        }) else {
            continue;
        };

        let Some(binding_id) = checker
            .semantic()
            .function_definitions(name)
            .iter()
            .copied()
            .find(|binding_id| checker.semantic().binding(*binding_id).span == name_span)
        else {
            continue;
        };

        if !reported.insert(binding_id) {
            continue;
        }

        let binding = checker.semantic().binding(binding_id);
        let Some(function_scope) =
            function_scope_for(checker, name, binding.scope, function.body.span)
        else {
            continue;
        };

        let positional = checker
            .facts()
            .function_positional_parameter_facts(function_scope);
        let required_arg_count = positional.required_arg_count();

        if required_arg_count <= 1 || positional.resets_positional_parameters() {
            continue;
        }
        if !function_is_called_without_arguments(checker, name, binding_id) {
            continue;
        }

        violations.push((
            trim_trailing_whitespace_span(function.span, checker.source()),
            name.to_string(),
        ));
    }

    for (span, name) in violations {
        checker.report(FunctionReferencesUnsetParam { name }, span);
    }
}

fn function_scope_for(
    checker: &Checker<'_>,
    name: &Name,
    enclosing_scope: ScopeId,
    body_span: Span,
) -> Option<ScopeId> {
    checker.semantic().scopes().iter().find_map(|scope| {
        let ScopeKind::Function(function) = &scope.kind else {
            return None;
        };
        (scope.parent == Some(enclosing_scope)
            && scope.span == body_span
            && function.contains_name(name))
        .then_some(scope.id)
    })
}

fn function_is_called_without_arguments(
    checker: &Checker<'_>,
    name: &Name,
    binding_id: BindingId,
) -> bool {
    let mut saw_relevant_call = false;

    for site in checker
        .semantic()
        .call_sites_for(name)
        .iter()
        .filter(|site| call_site_targets_binding(checker, name, binding_id, site))
    {
        saw_relevant_call = true;
        if site.arg_count > 0 {
            return false;
        }
    }

    saw_relevant_call
}

fn call_site_targets_binding(
    checker: &Checker<'_>,
    name: &Name,
    binding_id: BindingId,
    site: &shuck_semantic::CallSite,
) -> bool {
    let semantic = checker.semantic();
    let binding = semantic.binding(binding_id);
    let target_scope = semantic
        .ancestor_scopes(semantic.scope_at(site.span.start.offset))
        .find(|scope| {
            semantic
                .function_definitions(name)
                .iter()
                .any(|candidate| semantic.binding(*candidate).scope == *scope)
        });

    if target_scope != Some(binding.scope) {
        return false;
    }

    let site_offset = site.span.start.offset;
    if has_previous_redefinition(checker, binding_id) && site_offset <= binding.span.start.offset {
        return false;
    }

    match next_redefinition_offset(checker, binding_id) {
        Some(next_offset) => site_offset < next_offset,
        None => true,
    }
}

fn has_previous_redefinition(checker: &Checker<'_>, binding_id: BindingId) -> bool {
    checker
        .semantic_analysis()
        .overwritten_functions()
        .iter()
        .any(|overwritten| overwritten.second == binding_id)
}

fn next_redefinition_offset(checker: &Checker<'_>, binding_id: BindingId) -> Option<usize> {
    checker
        .semantic_analysis()
        .overwritten_functions()
        .iter()
        .find(|overwritten| overwritten.first == binding_id)
        .map(|overwritten| {
            checker
                .semantic()
                .binding(overwritten.second)
                .span
                .start
                .offset
        })
}

fn trim_trailing_whitespace_span(span: Span, source: &str) -> Span {
    let trimmed = span
        .slice(source)
        .trim_end_matches(|ch: char| ch.is_whitespace());
    Span::from_positions(span.start, span.start.advanced_by(trimmed))
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_functions_called_with_too_few_arguments() {
        let source = "\
#!/bin/sh
greet() { echo \"$1 $2\"; }
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "greet() { echo \"$1 $2\"; }"
        );
    }

    #[test]
    fn ignores_functions_called_with_enough_arguments() {
        let source = "\
#!/bin/sh
greet() { echo \"$1 $2\"; }
greet a b
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_functions_with_mixed_arity_calls() {
        let source = "\
#!/bin/sh
greet() { echo \"$1 $2\"; }
greet a b
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_functions_with_only_one_required_argument() {
        let source = "\
#!/bin/sh
greet() { echo \"$1\"; }
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_guarded_or_reset_positional_parameters() {
        let source = "\
#!/bin/sh
greet() {
  set -- hello
  echo \"${1:-default}\" \"$#\"
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn later_redefinitions_do_not_inherit_argumented_calls_from_earlier_bindings() {
        let source = "\
#!/bin/sh
greet() { echo \"$2\"; }
greet ok ok
greet() { echo \"$2\"; }
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "greet() { echo \"$2\"; }"
        );
    }

    #[test]
    fn ignores_plain_set_operands_that_reset_positional_parameters() {
        let source = "\
#!/bin/sh
greet() {
  set hello world
  echo \"$2\"
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn same_name_functions_in_different_scopes_do_not_mask_each_other() {
        let source = "\
#!/bin/sh
outer_without_args() {
  inner() { echo \"$1 $2\"; }
  inner
}

outer_with_args() {
  inner() { echo \"$1 $2\"; }
  inner ok yes
}

outer_without_args
outer_with_args
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "inner() { echo \"$1 $2\"; }"
        );
    }

    #[test]
    fn nested_command_substitutions_still_count_toward_required_arg_count() {
        let source = "\
#!/bin/sh
greet() {
  printf '%s\n' \"$(printf '%s' \"$1 $2\")\"
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "greet() {\n  printf '%s\n' \"$(printf '%s' \"$1 $2\")\"\n}"
        );
    }

    #[test]
    fn earlier_calls_in_same_scope_still_count_toward_mixed_arity() {
        let source = "\
#!/bin/sh
greet ok yes
greet() { echo \"$1 $2\"; }
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn nested_set_commands_still_count_as_positional_parameter_resets() {
        let source = "\
#!/bin/sh
greet() {
  printf '%s\n' 'hello world' | while read -r first second; do
    set -- \"$first\" \"$second\"
    printf '%s\n' \"$2\"
  done
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
