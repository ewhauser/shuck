use crate::{Checker, Rule, ShellDialect, Violation};

pub struct ErrexitTrapInSh;

impl Violation for ErrexitTrapInSh {
    fn rule() -> Rule {
        Rule::ErrexitTrapInSh
    }

    fn message(&self) -> String {
        "`set` trap inheritance flags are not portable in `sh` scripts".to_owned()
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
        .flat_map(|fact| {
            fact.options().set().into_iter().flat_map(|set| {
                set.errtrace_flag_spans()
                    .iter()
                    .chain(set.functrace_flag_spans().iter())
                    .copied()
            })
        })
        .collect::<Vec<_>>();

    checker.report_all(spans, || ErrexitTrapInSh);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_nonportable_trap_inheritance_flags_in_sh() {
        let source = "\
#!/bin/sh
set -E
set +T
set -ET
set -o errtrace
set +o functrace
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ErrexitTrapInSh));

        assert_eq!(diagnostics.len(), 4);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-E", "+T", "-ET", "-ET"]
        );
    }

    #[test]
    fn ignores_bash_shells() {
        let source = "\
#!/bin/bash
set -E
set -T
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ErrexitTrapInSh));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_positional_operands_after_double_dash() {
        let source = "\
#!/bin/sh
set -E -T -- +E +T
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ErrexitTrapInSh));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-E", "-T"]
        );
    }
}
