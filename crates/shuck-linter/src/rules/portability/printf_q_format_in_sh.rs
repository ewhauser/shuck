use crate::{Checker, Rule, ShellDialect, Violation};

pub struct PrintfQFormatInSh;

impl Violation for PrintfQFormatInSh {
    fn rule() -> Rule {
        Rule::PrintfQFormatInSh
    }

    fn message(&self) -> String {
        "printf `%q` is not portable in `sh` scripts".to_owned()
    }
}

pub fn printf_q_format_in_sh(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("printf"))
        .filter_map(|fact| {
            let printf = fact.options().printf()?;
            printf
                .uses_q_format
                .then_some(printf.format_word.map(|word| word.span))
                .flatten()
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || PrintfQFormatInSh);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_q_formats_in_sh() {
        let source = "\
#!/bin/sh
printf '%q\\n' foo
printf '%10q\\n' foo
printf '%*q\\n' 10 foo
printf '%%q\\n' foo
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::PrintfQFormatInSh));

        assert_eq!(diagnostics.len(), 3);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["'%q\\n'", "'%10q\\n'", "'%*q\\n'"]
        );
    }

    #[test]
    fn ignores_q_formats_in_bash() {
        let source = "\
#!/bin/bash
printf '%q\\n' foo
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::PrintfQFormatInSh));

        assert!(diagnostics.is_empty());
    }
}
