use shuck_ast::CaseTerminator;

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct BashCaseFallthrough;

impl Violation for BashCaseFallthrough {
    fn rule() -> Rule {
        Rule::BashCaseFallthrough
    }

    fn message(&self) -> String {
        "bash-style case fallthrough is not portable to this shell".to_owned()
    }
}

pub fn bash_case_fallthrough(checker: &mut Checker) {
    if !matches!(
        checker.shell(),
        ShellDialect::Sh | ShellDialect::Dash | ShellDialect::Ksh
    ) {
        return;
    }

    let spans = checker
        .facts()
        .case_items()
        .iter()
        .filter(|item| {
            matches!(
                item.terminator(),
                CaseTerminator::FallThrough | CaseTerminator::Continue
            )
        })
        .map(|item| {
            checker
                .facts()
                .command(item.command_id())
                .span_in_source(checker.source())
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || BashCaseFallthrough);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn ignores_bash_scripts() {
        let source = "#!/bin/bash\ncase $x in\n  a) : ;&\n  b) : ;;\nesac\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BashCaseFallthrough).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_ksh_scripts() {
        let source = "#!/bin/ksh\ncase $x in\n  a) : ;;&\n  b) : ;;\nesac\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BashCaseFallthrough).with_shell(ShellDialect::Ksh),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::BashCaseFallthrough);
    }

    #[test]
    fn reports_each_case_command_once() {
        let source = "#!/bin/sh\ncase $x in\n  a) : ;&\n  b) : ;;&\n  c) : ;;\nesac\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BashCaseFallthrough));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }
}
