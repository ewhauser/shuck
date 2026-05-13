use crate::facts::surface::rewrite_word_as_single_double_quoted_string;
use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct UnsetAssociativeArrayElement;

impl Violation for UnsetAssociativeArrayElement {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::UnsetAssociativeArrayElement
    }

    fn message(&self) -> String {
        "quote `unset` array-subscript operands so bracket text stays literal".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("quote the unset operand".to_owned())
    }
}

pub fn unset_associative_array_element(checker: &mut Checker) {
    let source = checker.source();
    let mut diagnostics = Vec::new();

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
                diagnostics.push((
                    operand.word().span,
                    rewrite_word_as_single_double_quoted_string(operand.word(), source, None),
                ));
            }
        }
    }

    for (span, replacement) in diagnostics {
        checker.report_diagnostic_dedup(
            Diagnostic::new(UnsetAssociativeArrayElement, span)
                .with_fix(Fix::unsafe_edit(Edit::replacement(replacement, span))),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

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

    #[test]
    fn applies_unsafe_fix_to_array_subscript_unset_operands() {
        let source = "\
#!/bin/bash
unset parts[\"one\"]
unset parts['two']
unset parts[$key]
unset parts[\"$key\"]
unset parts[\\\"four\\\"]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::UnsetAssociativeArrayElement),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 5);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
unset \"parts[one]\"
unset \"parts[two]\"
unset \"parts[${key}]\"
unset \"parts[${key}]\"
unset \"parts[\\\"four\\\"]\"
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn safe_fix_mode_leaves_unset_operands_unchanged() {
        let source = "#!/bin/bash\nunset parts[key]\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::UnsetAssociativeArrayElement),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixed_diagnostics.len(), 1);
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C108.sh").as_path(),
            &LinterSettings::for_rule(Rule::UnsetAssociativeArrayElement),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C108_fix_C108.sh", result);
        Ok(())
    }
}
