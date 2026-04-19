use crate::{Checker, Rule, ShellDialect, Violation};

pub struct LegacyArithmeticInSh;

impl Violation for LegacyArithmeticInSh {
    fn rule() -> Rule {
        Rule::LegacyArithmeticInSh
    }

    fn message(&self) -> String {
        "legacy `$[...]` arithmetic is not portable in `sh` scripts".to_owned()
    }
}

pub fn legacy_arithmetic_in_sh(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .legacy_arithmetic_fragments()
        .iter()
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();

    checker.report_all(spans, || LegacyArithmeticInSh);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_legacy_arithmetic_fragment() {
        let source = "#!/bin/sh\ni=$[$i+1]\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LegacyArithmeticInSh),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$[$i+1]");
    }

    #[test]
    fn anchors_on_spaced_legacy_arithmetic_fragment() {
        let source = "#!/bin/sh\ni=$[ $i - 1 ]\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LegacyArithmeticInSh),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$[ $i - 1 ]");
    }

    #[test]
    fn ignores_bash_scripts() {
        let source = "#!/bin/bash\ni=$[$i+1]\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LegacyArithmeticInSh).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }
}
