use crate::{Checker, CommandFactCompoundKind, Rule, ShellDialect, Violation};

pub struct Coproc;

impl Violation for Coproc {
    fn rule() -> Rule {
        Rule::Coproc
    }

    fn message(&self) -> String {
        "`coproc` is not portable in `sh` scripts".to_owned()
    }
}

pub fn coproc(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| match fact.compound_kind() {
            Some(CommandFactCompoundKind::Coproc) => {
                Some(fact.span_in_source(checker.source()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    checker.report_all(spans, || Coproc);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_full_coproc_command_span() {
        let source = "#!/bin/sh\ncoproc cat\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::Coproc));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "coproc cat");
    }

    #[test]
    fn reports_named_coproc_command_span() {
        let source = "#!/bin/sh\ncoproc pycoproc (python3 \"$pywrapper\")\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::Coproc));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "coproc pycoproc (python3 \"$pywrapper\")"
        );
    }

    #[test]
    fn ignores_bash_scripts() {
        let source = "#!/bin/bash\ncoproc cat\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::Coproc).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }
}
