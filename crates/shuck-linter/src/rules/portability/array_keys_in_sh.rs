use crate::{Checker, Rule, ShellDialect, Violation};

pub struct ArrayKeysInSh;

impl Violation for ArrayKeysInSh {
    fn rule() -> Rule {
        Rule::ArrayKeysInSh
    }

    fn message(&self) -> String {
        "`${!arr[*]}` array key expansion is not portable in `sh`".to_owned()
    }
}

pub fn array_keys_in_sh(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    // X018 owns the broader indirect-expansion wording when both rules are enabled.
    if checker.is_rule_enabled(Rule::IndirectExpansion) {
        return;
    }

    let spans = checker
        .facts()
        .indirect_expansion_fragments()
        .iter()
        .filter(|fragment| fragment.array_keys())
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ArrayKeysInSh);
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_snippet, test_snippet_at_path};
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_only_on_array_key_expansions() {
        let source = "\
#!/bin/sh
printf '%s\n' \"${!name}\" \"${!build_option_@}\" \"${!arr[*]}\" \"${!arr[@]}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ArrayKeysInSh));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${!arr[*]}", "${!arr[@]}"]
        );
    }

    #[test]
    fn ignores_array_key_expansions_in_bash() {
        let source = "printf '%s\\n' \"${!arr[*]}\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayKeysInSh).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn honors_shellcheck_shell_directive_in_plain_script() {
        let source = "\
# shellcheck shell=sh
printf '%s\\n' \"${!arr[*]}\"
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/x071-plain-script"),
            source,
            &LinterSettings::for_rule(Rule::ArrayKeysInSh),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "${!arr[*]}");
    }

    #[test]
    fn shellcheck_shell_directive_overrides_shebang_for_x071() {
        let source = "\
#!/bin/bash
# shellcheck shell=sh
printf '%s\\n' \"${!arr[*]}\"
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/x071-shell-override"),
            source,
            &LinterSettings::for_rule(Rule::ArrayKeysInSh),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "${!arr[*]}");
    }

    #[test]
    fn shellcheck_shell_directive_can_suppress_x071_under_sh_shebang() {
        let source = "\
#!/bin/sh
# shellcheck shell=bash
printf '%s\\n' \"${!arr[*]}\"
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/x071-shell-override"),
            source,
            &LinterSettings::for_rule(Rule::ArrayKeysInSh),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn defers_to_x018_when_both_indirect_expansion_rules_are_enabled() {
        let source = "printf '%s\\n' \"${!arr[*]}\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rules([Rule::ArrayKeysInSh, Rule::IndirectExpansion])
                .with_shell(ShellDialect::Sh),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::IndirectExpansion);
    }
}
