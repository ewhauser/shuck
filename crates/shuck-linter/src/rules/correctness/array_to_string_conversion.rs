use shuck_ast::Span;
use shuck_semantic::{Binding, BindingAttributes, BindingKind};

use crate::{Checker, ExpansionContext, Rule, Violation, WordFactContext};

pub struct ArrayToStringConversion;

impl Violation for ArrayToStringConversion {
    fn rule() -> Rule {
        Rule::ArrayToStringConversion
    }

    fn message(&self) -> String {
        "array values are flattened to a scalar string before later scalar use".to_owned()
    }
}

pub fn array_to_string_conversion(checker: &mut Checker) {
    let semantic = checker.semantic();

    let spans = semantic
        .bindings()
        .iter()
        .filter_map(|binding| {
            let context = binding_assignment_value_context(binding)?;
            if binding
                .attributes
                .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC)
            {
                return None;
            }

            let value = checker.facts().scalar_binding_value(binding.span)?;
            let value_fact = checker.facts().word_fact(value.span, context)?;
            if !uses_array_to_scalar_conversion_pattern(checker, binding, value_fact) {
                return None;
            }

            Some(binding.span)
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ArrayToStringConversion);
}

fn binding_assignment_value_context(binding: &Binding) -> Option<WordFactContext> {
    match binding.kind {
        BindingKind::Assignment | BindingKind::ParameterDefaultAssignment => Some(
            WordFactContext::Expansion(ExpansionContext::AssignmentValue),
        ),
        BindingKind::Declaration(_) => Some(WordFactContext::Expansion(
            ExpansionContext::DeclarationAssignmentValue,
        )),
        _ => None,
    }
}

fn uses_array_to_scalar_conversion_pattern(
    checker: &Checker<'_>,
    binding: &Binding,
    value_fact: &crate::WordFact<'_>,
) -> bool {
    value_fact
        .array_expansion_spans()
        .iter()
        .copied()
        .any(|span| {
            checker.semantic().references().iter().any(|reference| {
                reference.name == binding.name
                    && contains_span(span, reference.span)
                    && checker
                        .semantic()
                        .resolved_binding(reference.id)
                        .is_some_and(binding_is_array_like)
            })
        })
}

fn binding_is_array_like(binding: &Binding) -> bool {
    binding
        .attributes
        .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC)
        || binding.kind == BindingKind::ArrayAssignment
}

fn contains_span(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_true_array_to_scalar_conversions() {
        let source = "\
#!/bin/bash
exts=(txt pdf doc)
exts=\"${exts[*]}\"
items=(one two)
items=\"${items[0]}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["exts"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn ignores_assignments_without_prior_array_like_binding() {
        let source = "\
#!/bin/bash
name=base
name=\"${name}-suffix\"
other=\"${unknown:-fallback}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_shadowed_local_scalars_without_array_conversion_in_value() {
        let source = "\
#!/bin/bash
exts=(txt pdf)
f() {
  local exts=base
  exts=\"${exts}-suffix\"
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
