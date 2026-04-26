use crate::{Checker, CommandFactCompoundKind, Rule, ShellDialect, Violation};

pub struct StandaloneArithmetic;

impl Violation for StandaloneArithmetic {
    fn rule() -> Rule {
        Rule::StandaloneArithmetic
    }

    fn message(&self) -> String {
        "standalone `(( ))` arithmetic is not portable in `sh` scripts".to_owned()
    }
}

pub fn standalone_arithmetic(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| match fact.compound_kind() {
            Some(CommandFactCompoundKind::Arithmetic) => {
                Some(fact.span_in_source(checker.source()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    checker.report_all(spans, || StandaloneArithmetic);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn ignores_bash_scripts() {
        let source = "#!/bin/bash\n((count += 1))\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StandaloneArithmetic).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_arithmetic_expansions() {
        let source = "#!/bin/sh\nvalue=$((count + 1))\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StandaloneArithmetic),
        );

        assert!(diagnostics.is_empty());
    }
}
