use crate::{Checker, Rule, Violation};

pub struct ReadWithoutRaw;

impl Violation for ReadWithoutRaw {
    fn rule() -> Rule {
        Rule::ReadWithoutRaw
    }

    fn message(&self) -> String {
        "use `read -r` to keep backslashes literal".to_owned()
    }
}

pub fn read_without_raw(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("read"))
        .filter(|fact| {
            fact.options()
                .read()
                .is_some_and(|read| !read.uses_raw_input)
        })
        .filter_map(|fact| fact.body_name_word().map(|word| word.span))
        .collect::<Vec<_>>();

    checker.report_all(spans, || ReadWithoutRaw);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_plain_reads_and_nested_reads_without_raw_input() {
        let source = "\
#!/bin/sh
read line
command read line
builtin read line
printf '%s\\n' x | while read line; do :; done
value=\"$(read name)\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ReadWithoutRaw));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["read", "read", "read", "read", "read"]
        );
    }

    #[test]
    fn ignores_reads_with_raw_input() {
        let source = "\
#!/bin/bash
command read -r line
builtin read -r line
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ReadWithoutRaw).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }
}
