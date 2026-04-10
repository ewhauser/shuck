use rustc_hash::FxHashSet;
use shuck_ast::{Name, Span};
use shuck_semantic::{BindingId, ScopeId, ScopeKind};

use crate::{Checker, Rule, Violation};

pub struct FunctionCalledWithoutArgs {
    pub name: String,
}

impl Violation for FunctionCalledWithoutArgs {
    fn rule() -> Rule {
        Rule::FunctionCalledWithoutArgs
    }

    fn message(&self) -> String {
        format!("function `{}` is called without arguments", self.name)
    }
}

pub fn function_called_without_args(checker: &mut Checker) {
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
        let called_without_args = function_is_called_without_arguments(checker, name);

        if !positional.uses_positional_parameters() {
            continue;
        }
        if positional.resets_positional_parameters() {
            continue;
        }
        if !called_without_args {
            continue;
        }

        violations.push((
            trim_trailing_whitespace_span(function.span, checker.source()),
            name.to_string(),
        ));
    }

    for (span, name) in violations {
        checker.report(FunctionCalledWithoutArgs { name }, span);
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

fn function_is_called_without_arguments(checker: &Checker<'_>, name: &Name) -> bool {
    let mut saw_relevant_call = false;
    let mut saw_argument_call = false;

    for site in checker.semantic().call_sites_for(name) {
        saw_relevant_call = true;
        if site.arg_count > 0 {
            saw_argument_call = true;
            break;
        }
    }

    saw_relevant_call && !saw_argument_call
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
    fn reports_functions_called_without_arguments() {
        let source = "\
#!/bin/sh
greet() { echo \"$1\"; }
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "greet() { echo \"$1\"; }"
        );
    }

    #[test]
    fn ignores_functions_called_with_arguments() {
        let source = "\
#!/bin/sh
greet() { echo \"$@\"; }
greet world
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_functions_without_calls() {
        let source = "\
#!/bin/sh
greet() { echo \"$#\"; }
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_guarded_positional_parameters() {
        let source = "\
#!/bin/sh
greet() { echo \"${1:-default}\"; }
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_functions_that_reset_positional_parameters() {
        let source = "\
#!/bin/sh
greet() {
  set -- hello
  echo \"$1\"
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_functions_that_reset_positional_parameters_with_plain_operands() {
        let source = "\
#!/bin/sh
greet() {
  set hello
  echo \"$1\"
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_functions_that_use_variadic_positional_parameters() {
        let source = "\
#!/bin/sh
die() { echo \"$@\"; }
die
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "die() { echo \"$@\"; }");
    }

    #[test]
    fn reports_functions_that_use_argument_count() {
        let source = "\
#!/bin/sh
die() { echo \"$#\"; }
die
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "die() { echo \"$#\"; }");
    }

    #[test]
    fn ignores_call_sites_that_precede_the_definition_if_arguments_are_passed() {
        let source = "\
#!/bin/sh
main() {
  greet 1
  greet
}
main

greet() { echo \"$1\"; }
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn nested_functions_are_tracked_independently() {
        let source = "\
#!/bin/sh
outer() {
  inner() { echo \"$1\"; }
  inner
}
outer
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "inner() { echo \"$1\"; }"
        );
    }
}
