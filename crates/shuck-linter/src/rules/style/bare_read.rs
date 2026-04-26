use crate::{Checker, Rule, ShellDialect, Violation};

pub struct BareRead;

impl Violation for BareRead {
    fn rule() -> Rule {
        Rule::BareRead
    }

    fn message(&self) -> String {
        "give `read` a variable to store the input".to_owned()
    }
}

pub fn bare_read(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let source = checker.source();
    let spans = checker
        .facts()
        .structural_commands()
        .filter(|fact| fact.effective_name_is("read"))
        .filter(|fact| fact.wrappers().is_empty())
        .filter(|fact| fact.arena_body_args(source).is_empty())
        .filter_map(|fact| fact.arena_body_name_word(source).map(|word| word.span()))
        .collect::<Vec<_>>();

    checker.report_all(spans, || BareRead);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_plain_read_without_variables_only_in_posix_shells() {
        let source = "\
#!/bin/sh
read
read -r
read name
command read
builtin read
read -a arr
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BareRead));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["read"]
        );
    }

    #[test]
    fn ignores_wrapped_and_non_posix_reads() {
        let source = "\
#!/bin/bash
read
command read
builtin read
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BareRead).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }
}
