use shuck_semantic::BindingAttributes;

use crate::{Checker, Rule, Violation};

pub struct UnsetAssociativeArrayElement;

impl Violation for UnsetAssociativeArrayElement {
    fn rule() -> Rule {
        Rule::UnsetAssociativeArrayElement
    }

    fn message(&self) -> String {
        "quote associative-array unset targets as `'name[key]'` to keep keys literal".to_owned()
    }
}

pub fn unset_associative_array_element(checker: &mut Checker) {
    let semantic = checker.semantic();
    let mut spans = Vec::new();

    for fact in checker
        .facts()
        .structural_commands()
        .filter(|fact| fact.effective_name_is("unset"))
    {
        let Some(unset) = fact.options().unset() else {
            continue;
        };

        for operand in unset.operand_facts() {
            let Some(array_subscript) = operand.array_subscript() else {
                continue;
            };
            if !array_subscript.key_contains_quote() {
                continue;
            }

            let Some(visible) =
                semantic.visible_binding(array_subscript.name(), operand.word().span)
            else {
                continue;
            };
            if visible.attributes.contains(BindingAttributes::ASSOC) {
                spans.push(operand.word().span);
            }
        }
    }

    checker.report_all_dedup(spans, || UnsetAssociativeArrayElement);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_quoted_associative_unset_keys() {
        let source = "\
#!/bin/bash
declare -A parts
parts[one]=1
unset parts[\"one\"]
unset parts['two']
key=three
unset parts[\"$key\"]
unset parts[\\\"four\\\"]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnsetAssociativeArrayElement),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "parts[\"one\"]",
                "parts['two']",
                "parts[\"$key\"]",
                "parts[\\\"four\\\"]"
            ]
        );
    }

    #[test]
    fn ignores_indexed_or_safely_quoted_unset_operands() {
        let source = "\
#!/bin/bash
declare -a nums
declare -A parts
key=one
unset nums[\"1\"]
unset parts[$key]
unset 'parts[key]'
unset \"parts[key]\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnsetAssociativeArrayElement),
        );

        assert!(diagnostics.is_empty());
    }
}
