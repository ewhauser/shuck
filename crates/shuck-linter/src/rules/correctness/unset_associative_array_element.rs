use crate::{Checker, Rule, Violation};

pub struct UnsetAssociativeArrayElement;

impl Violation for UnsetAssociativeArrayElement {
    fn rule() -> Rule {
        Rule::UnsetAssociativeArrayElement
    }

    fn message(&self) -> String {
        "quote `unset` array-subscript operands so bracket text stays literal".to_owned()
    }
}

pub fn unset_associative_array_element(checker: &mut Checker) {
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
            if operand.array_subscript().is_some() {
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
    fn reports_array_subscript_unset_operands() {
        let source = "\
#!/bin/bash
declare -A parts
parts[one]=1
parts[two]=2
foo=1
unset parts[\"one\"]
unset parts['two']
key=three
declare -a nums
unset foo[1]
unset nums[1]
unset nums[\"1\"]
unset nums[$key]
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
                "foo[1]",
                "nums[1]",
                "nums[\"1\"]",
                "nums[$key]",
                "parts[\"$key\"]",
                "parts[\\\"four\\\"]"
            ]
        );
    }

    #[test]
    fn ignores_non_array_and_literal_unset_operands() {
        let source = "\
#!/bin/bash
declare -A parts
declare value=one
unset plain
unset value
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
