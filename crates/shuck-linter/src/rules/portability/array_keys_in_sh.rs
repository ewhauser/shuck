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
    use crate::test::test_snippet;
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
}
