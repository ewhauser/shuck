use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct ReadWithoutRaw;

impl Violation for ReadWithoutRaw {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::ReadWithoutRaw
    }

    fn message(&self) -> String {
        "use `read -r` to keep backslashes literal".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("add `-r` to `read`".to_owned())
    }
}

pub fn read_without_raw(checker: &mut Checker) {
    let diagnostics = checker
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
        .map(|span| {
            Diagnostic::new(ReadWithoutRaw, span)
                .with_fix(Fix::unsafe_edit(Edit::insertion(span.end.offset, " -r")))
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect};

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

    #[test]
    fn applies_unsafe_fix_by_adding_raw_read_flag() {
        let source = "#!/bin/sh\nread line\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::ReadWithoutRaw),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/sh\nread -r line\n");
        assert!(result.fixed_diagnostics.is_empty());
    }
}
