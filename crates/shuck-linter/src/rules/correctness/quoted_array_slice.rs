use rustc_hash::FxHashSet;

use crate::{Checker, ExpansionContext, FactSpan, Rule, Violation};

pub struct QuotedArraySlice;

impl Violation for QuotedArraySlice {
    fn rule() -> Rule {
        Rule::QuotedArraySlice
    }

    fn message(&self) -> String {
        "quoted array-slice expansions collapse into one string value".to_owned()
    }
}

pub fn quoted_array_slice(checker: &mut Checker) {
    let scalar_assignment_value_spans = checker
        .facts()
        .binding_values()
        .values()
        .filter_map(|binding_value| binding_value.scalar_word())
        .map(|word| FactSpan::new(word.span))
        .collect::<FxHashSet<_>>();

    let spans = [
        ExpansionContext::AssignmentValue,
        ExpansionContext::DeclarationAssignmentValue,
    ]
    .into_iter()
    .flat_map(|context| checker.facts().expansion_word_facts(context))
    .filter(|fact| scalar_assignment_value_spans.contains(&fact.key()))
    .filter(|fact| fact.has_quoted_all_elements_array_slice())
    .map(|fact| fact.span())
    .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || QuotedArraySlice);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_quoted_array_slice_assignments_into_scalar_bindings() {
        let source = "\
#!/bin/bash
x=\"${@:5}\"
y=\"prefix${@:2}suffix\"
declare z=\"${arr[@]:1}\"
readonly q=\"${arr[@]:1:2}\"
f() { local nested=\"${@:3}\"; }
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedArraySlice));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "\"${@:5}\"",
                "\"prefix${@:2}suffix\"",
                "\"${arr[@]:1}\"",
                "\"${arr[@]:1:2}\"",
                "\"${@:3}\"",
            ]
        );
    }

    #[test]
    fn ignores_unquoted_non_slice_and_compound_array_assignments() {
        let source = "\
#!/bin/bash
x=${@:5}
x=\"$@\"
x=\"${@:-fallback}\"
x=\"${arr[*]:1}\"
arr=(\"${@:2}\")
declare -a packed=(\"${arr[@]:1}\")
printf '%s\\n' \"${@:2}\"
if [ \"${arr[@]:1}\" = foo ]; then :; fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedArraySlice));

        assert!(diagnostics.is_empty());
    }
}
