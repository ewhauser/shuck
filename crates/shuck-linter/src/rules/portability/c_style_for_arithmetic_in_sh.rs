use crate::{Checker, Rule, ShellDialect, Violation};

pub struct CStyleForArithmeticInSh;

impl Violation for CStyleForArithmeticInSh {
    fn rule() -> Rule {
        Rule::CStyleForArithmeticInSh
    }

    fn message(&self) -> String {
        "C-style `for ((...))` arithmetic operators are not portable in `sh` scripts".to_owned()
    }
}

pub fn c_style_for_arithmetic_in_sh(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .arithmetic_for_update_operator_spans()
        .to_vec();

    checker.report_all(spans, || CStyleForArithmeticInSh);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_update_operators_inside_c_style_for() {
        let source = "#!/bin/sh\nfor ((++i; j < 3; k--)); do :; done\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["++", "--"]
        );
    }

    #[test]
    fn ignores_bash_scripts() {
        let source = "#!/bin/bash\nfor ((++i; j < 3; k--)); do :; done\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }
}
