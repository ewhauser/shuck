use crate::{Checker, Rule, ShellDialect, Violation, static_word_text};

pub struct ErrexitTrapInSh;

impl Violation for ErrexitTrapInSh {
    fn rule() -> Rule {
        Rule::ErrexitTrapInSh
    }

    fn message(&self) -> String {
        "`set -E` is not portable in `sh` scripts".to_owned()
    }
}

pub fn errexit_trap_in_sh(checker: &mut Checker) {
    if checker.shell() != ShellDialect::Sh {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("set"))
        .filter(|fact| {
            fact.options()
                .set()
                .is_some_and(|set| set.errtrace_change.is_some())
        })
        .flat_map(|fact| {
            fact.body_args().iter().filter_map(|word| {
                let text = static_word_text(word, checker.source())?;
                (text == "errtrace"
                    || (text.starts_with(['-', '+']) && text[1..].contains('E')))
                .then_some(word.span)
            })
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ErrexitTrapInSh);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_errtrace_options_in_sh() {
        let source = "\
#!/bin/sh
set -E
set -eE
set -o errtrace
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ErrexitTrapInSh));

        assert_eq!(diagnostics.len(), 3);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-E", "-eE", "errtrace"]
        );
    }

    #[test]
    fn ignores_bash_shells() {
        let source = "\
#!/bin/bash
set -E
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ErrexitTrapInSh));

        assert!(diagnostics.is_empty());
    }
}
