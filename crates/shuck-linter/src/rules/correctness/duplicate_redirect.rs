use crate::{Checker, Rule, ShellDialect, Violation};

pub struct DuplicateRedirect;

impl Violation for DuplicateRedirect {
    fn rule() -> Rule {
        Rule::DuplicateRedirect
    }

    fn message(&self) -> String {
        "multiple redirects target the same descriptor".to_owned()
    }
}

pub fn duplicate_redirect(checker: &mut Checker) {
    if checker.shell() == ShellDialect::Zsh {
        return;
    }

    checker.report_all(
        checker.facts().command_facts().duplicate_redirect_spans(),
        || DuplicateRedirect,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_each_redirect_in_an_overridden_descriptor_group() {
        let source = "\
#!/bin/bash
: >a >b
: 2>a 2>b
: <a <b
: &>a >b
: &>a &>b
: &>>a 2>b
: &>> a 2>b
: >&file 2>err
: 2>&file 2>err
: 2>a 2>&1
: 2>&1 2>b
: <in 0<&3
: 3<>a 3>b
: 1<>a 1>&2
: 1>a 2>&1 1>b 2>c
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DuplicateRedirect));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                ">", ">", ">", ">", "<", "<", ">", ">", ">", ">", ">>", ">", ">>", ">", ">&", ">",
                ">&", ">", ">", ">&", ">&", ">", "<", "<&", "<>", ">", "<>", ">&", ">", ">&", ">",
                ">",
            ]
        );
    }

    #[test]
    fn ignores_distinct_descriptors_and_descriptor_duplication() {
        let source = "\
#!/bin/bash
: >a 2>b
: <>a >b
: >a <>b
: 1>out 2>&1 1>other
: >&- >a
: >&1 2>err
exec {fd}>a {fd}>b
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DuplicateRedirect));

        assert!(diagnostics.is_empty());
    }
}
