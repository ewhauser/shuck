use crate::{Checker, Rule, ShellDialect, Violation};

pub struct IndirectExpansion;

impl Violation for IndirectExpansion {
    fn rule() -> Rule {
        Rule::IndirectExpansion
    }

    fn message(&self) -> String {
        "indirect expansion is not portable in `sh`".to_owned()
    }
}

pub fn indirect_expansion(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let specific_array_keys_rule_enabled = checker.is_rule_enabled(Rule::ArrayKeysInSh);

    let spans = checker
        .facts()
        .indirect_expansion_fragments()
        .iter()
        .filter(|fragment| !(specific_array_keys_rule_enabled && fragment.array_keys()))
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || IndirectExpansion);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_indirect_expansions_prefix_matches_and_array_key_forms() {
        let source = "\
#!/bin/sh
printf '%s\n' \"${!name}\" \"${!name:-fallback}\" \"${!build_option_@}\" \"${!arr[*]}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::IndirectExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "${!name}",
                "${!name:-fallback}",
                "${!build_option_@}",
                "${!arr[*]}",
            ]
        );
    }

    #[test]
    fn ignores_indirect_expansion_in_bash() {
        let source = "printf '%s\n' \"${!name}\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::IndirectExpansion).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn leaves_array_key_expansions_to_x071_when_enabled() {
        let source = "\
#!/bin/sh
printf '%s\n' \"${!name}\" \"${!arr[*]}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rules([Rule::IndirectExpansion, Rule::ArrayKeysInSh])
                .with_shell(ShellDialect::Sh),
        );

        assert_eq!(diagnostics.len(), 2);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.span.slice(source) == "${!name}"
                    && diagnostic.rule == Rule::IndirectExpansion)
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.span.slice(source) == "${!arr[*]}"
                    && diagnostic.rule == Rule::ArrayKeysInSh)
        );
    }
}
