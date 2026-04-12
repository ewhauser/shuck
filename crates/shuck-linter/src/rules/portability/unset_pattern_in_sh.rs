use crate::{Checker, Rule, ShellDialect, Violation};

pub struct UnsetPatternInSh;

impl Violation for UnsetPatternInSh {
    fn rule() -> Rule {
        Rule::UnsetPatternInSh
    }

    fn message(&self) -> String {
        "pattern-based `unset` is not portable in `sh`".to_owned()
    }
}

pub fn unset_pattern_in_sh(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    // X018 owns the broader indirect-expansion wording when both rules are enabled.
    if checker.is_rule_enabled(Rule::IndirectExpansion) {
        return;
    }

    let spans = checker
        .facts()
        .structural_commands()
        .filter(|fact| fact.effective_name_is("unset"))
        .filter_map(|fact| fact.options().unset())
        .flat_map(|unset| unset.prefix_match_operand_spans().iter().copied())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || UnsetPatternInSh);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_prefix_matching_unset_operands_in_sh() {
        let source = "\
#!/bin/sh
unset -v \"${!prefix_@}\" x${!prefix_*} \"${!name}\" \"${!arr[@]}\"
unset -f \"${!func_@}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnsetPatternInSh));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${!prefix_@}", "${!prefix_*}", "${!func_@}"]
        );
    }

    #[test]
    fn ignores_non_prefix_indirect_unset_operands() {
        let source = "\
#!/bin/sh
unset -v \"${!name}\" \"${!arr[@]}\"
unset value
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnsetPatternInSh));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_pattern_unset_operands_in_bash() {
        let source = "unset -v \"${!prefix_@}\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnsetPatternInSh).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn defers_to_x018_when_both_indirect_expansion_rules_are_enabled() {
        let source = "unset -v \"${!prefix_@}\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rules([Rule::UnsetPatternInSh, Rule::IndirectExpansion])
                .with_shell(ShellDialect::Sh),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::IndirectExpansion);
    }
}
