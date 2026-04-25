use rustc_hash::FxHashMap;
use shuck_ast::{ArrayElem, Assignment, AssignmentValue, Span};
use shuck_semantic::ReferenceKind;

use crate::{Checker, Rule, Violation};

pub struct LocalCrossReference;

impl Violation for LocalCrossReference {
    fn rule() -> Rule {
        Rule::LocalCrossReference
    }

    fn message(&self) -> String {
        "assignment is reused later in the same declaration".to_owned()
    }
}

pub fn local_cross_reference(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| declaration_cross_reference_spans(checker, fact))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || LocalCrossReference);
}

fn declaration_cross_reference_spans(
    checker: &Checker<'_>,
    fact: crate::CommandFactRef<'_, '_>,
) -> Vec<Span> {
    let Some(declaration) = fact.declaration() else {
        return Vec::new();
    };

    let mut seen_targets = FxHashMap::default();
    let mut spans = Vec::new();

    for assignment in declaration.assignment_operands.iter().copied() {
        for value_span in assignment_value_spans(assignment) {
            for reference in checker.semantic().references().iter().filter(|reference| {
                reference.kind != ReferenceKind::DeclarationName
                    && contains_span(value_span, reference.span)
            }) {
                if let Some(previous_span) = seen_targets.get(reference.name.as_str()) {
                    spans.push(*previous_span);
                }
            }
        }

        seen_targets.insert(
            assignment.target.name.as_str().to_owned(),
            assignment.target.name_span,
        );
    }

    spans
}

fn assignment_value_spans(assignment: &Assignment) -> Vec<Span> {
    match &assignment.value {
        AssignmentValue::Scalar(word) => vec![word.span],
        AssignmentValue::Compound(array) => array
            .elements
            .iter()
            .flat_map(array_element_spans)
            .collect(),
    }
}

fn array_element_spans(element: &ArrayElem) -> Vec<Span> {
    match element {
        ArrayElem::Sequential(word) => vec![word.span],
        ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
            vec![key.span(), value.span]
        }
    }
}

fn contains_span(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_prior_assignment_names_in_declarations() {
        let source = "\
#!/bin/sh
local a=1 b=$a c=$b
declare x=1 y=$(printf '%s' \"$x\")
readonly p=1 q=(\"$p\")
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::LocalCrossReference));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["a", "b", "x", "p"]
        );
    }

    #[test]
    fn prefers_the_most_recent_prior_assignment_for_reused_names() {
        let source = "\
#!/bin/sh
local a=1 a=2 c=$a
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::LocalCrossReference));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.start.offset,
            source.find("a=2").unwrap()
        );
    }

    #[test]
    fn ignores_associative_array_keys_inside_arithmetic_subscripts() {
        let source = "\
#!/bin/bash
f() {
  declare -A box=([m_width]=1 [mem_col]=5)
  local m_width=1 mem_line=$((box[mem_col]+box[m_width]))
}
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::LocalCrossReference));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_associative_array_keys_after_arithmetic_writes() {
        let source = "\
#!/bin/bash
f() {
  declare -A box=([key]=1)
  (( box[seed] = 1 ))
  local key=1 value=$((box[key]))
}
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::LocalCrossReference));

        assert!(diagnostics.is_empty());
    }
}
