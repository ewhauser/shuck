use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct FindOrWithoutGrouping;

impl Violation for FindOrWithoutGrouping {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::FindOrWithoutGrouping
    }

    fn message(&self) -> String {
        "`find` alternatives joined with `-o` should be grouped before applying actions".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("group the action-bearing find branch".to_owned())
    }
}

pub fn find_or_without_grouping(checker: &mut Checker) {
    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| fact.options().find())
        .flat_map(|find| {
            find.or_without_grouping_spans()
                .iter()
                .map(|span| {
                    (
                        *span,
                        find.or_without_grouping_fix_spans()
                            .iter()
                            .find(|fix| fix.diagnostic_span == *span)
                            .copied(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    for (span, fix_span) in diagnostics {
        let diagnostic = Diagnostic::new(FindOrWithoutGrouping, span);
        if let Some(fix_span) = fix_span {
            checker.report_diagnostic_dedup(diagnostic.with_fix(Fix::safe_edits([
                Edit::insertion(fix_span.branch_start.start.offset, "\\( "),
                Edit::insertion(fix_span.action_span.end.offset, " \\)"),
            ])));
        } else {
            checker.report_diagnostic_dedup(diagnostic);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_ungrouped_or_when_action_is_only_in_later_branch() {
        let source = "find . -name a -o -name b -print\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FindOrWithoutGrouping),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "-print");
    }

    #[test]
    fn ignores_when_right_branch_is_action_only() {
        let source = "find . -name a -o -print\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FindOrWithoutGrouping),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_top_level_or_even_when_one_branch_uses_grouping() {
        let source = "find . \\( -name a \\) -o -name b -print\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FindOrWithoutGrouping),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "-print");
    }

    #[test]
    fn reports_ungrouped_or_inside_grouped_subexpressions() {
        let source = "find . \\( -name a -o -name b -print \\)\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FindOrWithoutGrouping),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "-print");
    }

    #[test]
    fn reports_ungrouped_or_when_find_exec_branch_has_a_predicate() {
        let source = "find . -name a -o -name b -exec rm -f {} \\;\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FindOrWithoutGrouping),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "-exec");
    }

    #[test]
    fn reports_ungrouped_or_across_multiple_exec_branches() {
        let source = "find . -name a -o -name b -o -name c -exec rm -f {} \\;\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FindOrWithoutGrouping),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "-exec");
    }

    #[test]
    fn ignores_grouped_or_and_explicit_and() {
        let source = "\
find . \\( -name a -o -name b \\) -exec rm -f {} \\;
find . -name a -o -name b -a -exec rm -f {} \\;
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FindOrWithoutGrouping),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_when_an_earlier_branch_already_has_an_action() {
        let source = "find . -name a -print -o -name b -print\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FindOrWithoutGrouping),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_actions_inside_grouped_right_branches() {
        let source = "find . \\( -name a -o \\( -name b -exec rm -f {} \\; \\) \\) -print\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FindOrWithoutGrouping),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_common_prune_or_print_patterns() {
        let source = "find . -path './docs' -prune -o -type f -print\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FindOrWithoutGrouping),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_operator_like_predicate_arguments() {
        let source = "find . -name '-o' -type f -print\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FindOrWithoutGrouping),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_actions_after_find_comma_operator() {
        let source = "find . -name a -o -name b , -print\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FindOrWithoutGrouping),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_safe_fix_to_action_branches_without_action_arguments() {
        let source = "\
find . -name a -o -name b -print
find . -name a -o -name b -delete
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::FindOrWithoutGrouping),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "\
find . -name a -o \\( -name b -print \\)
find . -name a -o \\( -name b -delete \\)
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn applies_safe_fix_to_negated_action_branches_from_operator_start() {
        let source = "\
find . -name a -o ! -name b -print
find . -name a -o -not -name b -delete
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::FindOrWithoutGrouping),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "\
find . -name a -o \\( ! -name b -print \\)
find . -name a -o \\( -not -name b -delete \\)
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn withholds_fix_for_exec_actions_that_have_arguments() {
        let source = "find . -name a -o -name b -exec rm -f {} \\;\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::FindOrWithoutGrouping),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixed_diagnostics.len(), 1);
    }

    #[test]
    fn snapshots_safe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C103.sh").as_path(),
            &LinterSettings::for_rule(Rule::FindOrWithoutGrouping),
            Applicability::Safe,
        )?;

        assert_diagnostics_diff!("C103_fix_C103.sh", result);
        Ok(())
    }
}
