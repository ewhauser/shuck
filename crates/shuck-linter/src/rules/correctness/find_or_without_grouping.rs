use crate::{Checker, Rule, Violation};

pub struct FindOrWithoutGrouping;

impl Violation for FindOrWithoutGrouping {
    fn rule() -> Rule {
        Rule::FindOrWithoutGrouping
    }

    fn message(&self) -> String {
        "`find` alternatives joined with `-o` should be grouped before applying actions".to_owned()
    }
}

pub fn find_or_without_grouping(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("find"))
        .filter_map(|fact| fact.options().find())
        .flat_map(|find| find.or_without_grouping_spans().iter().copied())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || FindOrWithoutGrouping);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

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
    fn reports_ungrouped_or_when_right_branch_is_action_only() {
        let source = "find . -name a -o -print\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FindOrWithoutGrouping),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "-print");
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
}
