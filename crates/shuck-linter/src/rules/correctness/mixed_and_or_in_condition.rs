use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct MixedAndOrInCondition;

impl Violation for MixedAndOrInCondition {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::MixedAndOrInCondition
    }

    fn message(&self) -> String {
        "mixing `&&` and `||` inside `[[ ... ]]` needs parentheses to stay clear".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("add parentheses to make the logical grouping explicit".to_owned())
    }
}

pub fn mixed_and_or_in_condition(checker: &mut Checker) {
    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|command| command.conditional())
        .flat_map(|conditional| conditional.mixed_logical_operators().iter())
        .map(|operator| {
            let edits = operator
                .grouped_subexpression_spans()
                .iter()
                .flat_map(|span| {
                    [
                        Edit::insertion(span.start.offset, "( "),
                        Edit::insertion(span.end.offset, " )"),
                    ]
                })
                .collect::<Vec<_>>();

            Diagnostic::new(MixedAndOrInCondition, operator.operator_span())
                .with_fix(Fix::safe_edits(edits))
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_ungrouped_mixed_logical_operators_in_double_brackets() {
        let source = "\
#!/bin/bash
[[ -n $a && -n $b || -n $c ]]
[[ -n $a || -n $b && -n $c ]]
[[ ( -n $a && -n $b || -n $c ) && -n $d ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MixedAndOrInCondition),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["||", "||", "||"]
        );
    }

    #[test]
    fn ignores_grouped_or_single_operator_logical_conditions() {
        let source = "\
#!/bin/bash
[[ -n $a && ( -n $b || -n $c ) ]]
[[ ( -n $a && -n $b ) || -n $c ]]
[[ -n $a && -n $b && -n $c ]]
[[ -n $a || -n $b || -n $c ]]
[ -n \"$a\" -a -n \"$b\" -o -n \"$c\" ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MixedAndOrInCondition),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_safe_fix_to_mixed_logical_conditions() {
        let source = "\
#!/bin/bash
[[ -n $a && -n $b || -n $c ]]
[[ -n $a || -n $b && -n $c ]]
[[ ( -n $a && -n $b || -n $c ) && -n $d ]]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::MixedAndOrInCondition),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
[[ ( -n $a && -n $b ) || -n $c ]]
[[ -n $a || ( -n $b && -n $c ) ]]
[[ ( ( -n $a && -n $b ) || -n $c ) && -n $d ]]
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn applies_one_safe_fix_with_multiple_grouping_edits_when_both_sides_need_parentheses() {
        let source = "\
#!/bin/bash
[[ -n $a && -n $b || -n $c && -n $d ]]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::MixedAndOrInCondition),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
[[ ( -n $a && -n $b ) || ( -n $c && -n $d ) ]]
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_grouped_or_single_operator_conditions_unchanged_when_fixing() {
        let source = "\
#!/bin/bash
[[ -n $a && ( -n $b || -n $c ) ]]
[[ ( -n $a && -n $b ) || -n $c ]]
[[ -n $a && -n $b && -n $c ]]
[[ -n $a || -n $b || -n $c ]]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::MixedAndOrInCondition),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_safe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C088.sh").as_path(),
            &LinterSettings::for_rule(Rule::MixedAndOrInCondition),
            Applicability::Safe,
        )?;

        assert_diagnostics_diff!("C088_fix_C088.sh", result);
        Ok(())
    }
}
