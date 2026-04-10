use crate::{Checker, Rule, Violation};

pub struct DeprecatedTempfileCommand;

impl Violation for DeprecatedTempfileCommand {
    fn rule() -> Rule {
        Rule::DeprecatedTempfileCommand
    }

    fn message(&self) -> String {
        "use `mktemp` instead of `tempfile`".to_owned()
    }
}

pub fn deprecated_tempfile_command(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("tempfile") && fact.wrappers().is_empty())
        .filter_map(|fact| fact.body_name_word().map(|word| word.span))
        .collect::<Vec<_>>();

    checker.report_all(spans, || DeprecatedTempfileCommand);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_plain_tempfile_invocations() {
        let source = "\
#!/bin/sh
tempfile -n \"$TMPDIR/Xauthority\"
tempfile
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::DeprecatedTempfileCommand),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["tempfile", "tempfile"]
        );
    }

    #[test]
    fn ignores_wrapped_tempfile_invocations() {
        let source = "\
#!/bin/sh
command tempfile -n \"$TMPDIR/Xauthority\"
sudo tempfile -n \"$TMPDIR/Xauthority\"
alias tempfile=mktemp
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::DeprecatedTempfileCommand),
        );

        assert!(diagnostics.is_empty());
    }
}
