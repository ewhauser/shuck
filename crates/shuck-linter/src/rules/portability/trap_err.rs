use crate::{Checker, Rule, ShellDialect, Violation, static_word_text};

pub struct TrapErr;

impl Violation for TrapErr {
    fn rule() -> Rule {
        Rule::TrapErr
    }

    fn message(&self) -> String {
        "`ERR` traps are not portable in `sh` scripts".to_owned()
    }
}

pub fn trap_err(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("trap"))
        .flat_map(|fact| {
            fact.body_args().iter().skip(1).filter_map(|word| {
                static_word_text(word, checker.source())
                    .is_some_and(|text| text == "ERR")
                    .then_some(word.span)
            })
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || TrapErr);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_err_traps_in_sh() {
        let source = "\
#!/bin/sh
trap 'echo hi' ERR
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrapErr));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "ERR");
    }

    #[test]
    fn reports_err_in_trap_listing_modes() {
        let source = "\
#!/bin/sh
trap -p ERR
trap -l ERR
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrapErr));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["ERR", "ERR"]
        );
    }

    #[test]
    fn ignores_lone_err_action() {
        let source = "\
#!/bin/sh
trap ERR
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrapErr));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_bash_shells() {
        let source = "\
#!/bin/bash
trap 'echo hi' ERR
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrapErr));

        assert!(diagnostics.is_empty());
    }
}
