use shuck_semantic::{Binding, BindingAttributes, BindingKind};

use crate::facts::leading_literal_word_prefix;
use crate::{Checker, Rule, Violation};

pub struct AppendToArrayAsString;

impl Violation for AppendToArrayAsString {
    fn rule() -> Rule {
        Rule::AppendToArrayAsString
    }

    fn message(&self) -> String {
        "appending a string to an array with `+=` merges into an element; use `+=(...)`".to_owned()
    }
}

pub fn append_to_array_as_string(checker: &mut Checker) {
    let source = checker.source();
    let semantic = checker.semantic();

    let spans = semantic
        .bindings()
        .iter()
        .filter_map(|binding| {
            if binding.kind != BindingKind::AppendAssignment {
                return None;
            }
            if binding
                .attributes
                .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC)
            {
                return None;
            }

            let prior_binding = previous_visible_binding(semantic, binding)?;
            if !binding_is_array_like(prior_binding) {
                return None;
            }

            let value = checker.facts().binding_value(binding.id)?.scalar_word()?;
            leading_literal_word_prefix(value, source)
                .starts_with(' ')
                .then_some(binding.span)
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || AppendToArrayAsString);
}

fn previous_visible_binding<'a>(
    semantic: &'a shuck_semantic::SemanticModel,
    binding: &Binding,
) -> Option<&'a Binding> {
    for scope_id in semantic.ancestor_scopes(binding.scope) {
        let Some(scope) = semantic.scopes().iter().find(|scope| scope.id == scope_id) else {
            continue;
        };
        let Some(candidates) = scope.bindings.get(&binding.name) else {
            continue;
        };

        for candidate_id in candidates.iter().rev() {
            let candidate = semantic.binding(*candidate_id);

            if candidate.id == binding.id {
                continue;
            }
            if candidate.span.start.offset <= binding.span.start.offset {
                return Some(candidate);
            }
        }
    }

    None
}

fn binding_is_array_like(binding: &Binding) -> bool {
    binding
        .attributes
        .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC)
        || binding.kind == BindingKind::ArrayAssignment
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_string_appends_on_array_bindings() {
        let source = "\
#!/bin/bash
items=(one)
items+=\" two\"
declare -a flags=(--first)
flags+=\" ${extra}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AppendToArrayAsString),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["items", "flags"]
        );
    }

    #[test]
    fn ignores_non_array_and_element_appends() {
        let source = "\
#!/bin/bash
name=base
name+=\" suffix\"
items=(one)
items+=(\" two\")
items[0]+=\" tail\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AppendToArrayAsString),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_shadowed_local_scalars_when_outer_binding_is_array() {
        let source = "\
#!/bin/bash
arr=(one)
f() {
  local arr=base
  arr+=\" two\"
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AppendToArrayAsString),
        );

        assert!(diagnostics.is_empty());
    }
}
