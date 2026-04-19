use super::trap_common::parse_trap_args;
use crate::{Checker, Rule, ShellDialect, Violation, static_word_text};

pub struct TrapErr;

const NONPORTABLE_TRAP_PSEUDO_SIGNALS: &[&str] = &["ERR", "DEBUG", "RETURN"];

impl Violation for TrapErr {
    fn rule() -> Rule {
        Rule::TrapErr
    }

    fn message(&self) -> String {
        "`ERR`, `DEBUG`, and `RETURN` traps are not portable in `sh` scripts".to_owned()
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
        .flat_map(|fact| trap_pseudo_signal_spans(fact.body_args(), checker.source()))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || TrapErr);
}

fn trap_pseudo_signal_spans(args: &[&shuck_ast::Word], source: &str) -> Vec<shuck_ast::Span> {
    let Some(parsed) = parse_trap_args(args, source) else {
        return Vec::new();
    };

    parsed
        .signal_words
        .iter()
        .filter_map(|word| {
            static_word_text(word, source).and_then(|text| {
                NONPORTABLE_TRAP_PSEUDO_SIGNALS
                    .iter()
                    .any(|signal| text.eq_ignore_ascii_case(signal))
                    .then_some(word.span)
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_nonportable_trap_pseudo_signals_in_sh() {
        let source = "\
#!/bin/sh
trap 'echo hi' ERR
trap 'echo debug' DEBUG
trap 'echo return' RETURN
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrapErr));

        assert_eq!(diagnostics.len(), 3);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["ERR", "DEBUG", "RETURN"]
        );
    }

    #[test]
    fn reports_err_in_trap_listing_modes() {
        let source = "\
#!/bin/sh
trap -p ERR
trap -l DEBUG
trap -lp RETURN
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrapErr));

        assert_eq!(diagnostics.len(), 3);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["ERR", "DEBUG", "RETURN"]
        );
    }

    #[test]
    fn reports_actionless_pseudo_signal_reset() {
        let source = "\
#!/bin/sh
trap ERR
trap -- DEBUG
trap RETURN
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrapErr));

        assert_eq!(diagnostics.len(), 3);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["ERR", "DEBUG", "RETURN"]
        );
    }

    #[test]
    fn reports_case_insensitive_pseudo_signal_names() {
        let source = "\
#!/bin/sh
trap 'echo hi' err
trap -- DeBuG
trap return
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrapErr));

        assert_eq!(diagnostics.len(), 3);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["err", "DeBuG", "return"]
        );
    }

    #[test]
    fn ignores_pseudo_signal_action_after_double_dash() {
        let source = "\
#!/bin/sh
trap -- DEBUG EXIT
trap -- 'echo hi' RETURN
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrapErr));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "RETURN");
    }

    #[test]
    fn ignores_portable_exit_traps() {
        let source = "\
#!/bin/sh
trap 'echo hi' EXIT
trap -p EXIT
trap -- EXIT
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
