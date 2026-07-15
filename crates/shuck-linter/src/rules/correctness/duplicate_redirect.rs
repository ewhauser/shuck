use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, ShellDialect, Violation};

pub struct DuplicateRedirect;

impl Violation for DuplicateRedirect {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::DuplicateRedirect
    }

    fn message(&self) -> String {
        "multiple redirects target the same descriptor".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("delete the overridden redirect".to_owned())
    }
}

pub fn duplicate_redirect(checker: &mut Checker) {
    if checker.shell() == ShellDialect::Zsh {
        return;
    }

    for fact in checker.facts().command_facts().duplicate_redirect_facts() {
        let diagnostic = Diagnostic::new(DuplicateRedirect, fact.diagnostic_span());
        let diagnostic = match fact.deletion_span() {
            Some(span) => diagnostic.with_fix(Fix::unsafe_edit(Edit::deletion(span))),
            None => diagnostic,
        };
        checker.report_diagnostic(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

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
: 1>&file 2>err
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
                ">&", ">", ">&", ">", ">", ">&", ">&", ">", "<", "<&", "<>", ">", "<>", ">&", ">",
                ">&", ">", ">",
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

    #[test]
    fn keeps_overwriting_redirect_when_consumed_by_later_descriptor_copy() {
        let source = "\
#!/bin/bash
: 1>a 1>b 2>&1
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DuplicateRedirect));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.start.offset,
            source.find('>').expect("first redirect")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_fully_overridden_redirects() {
        let source = "#!/bin/sh\n: >first >second\n: 2>a 2>b 2>c\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::DuplicateRedirect),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(result.fixed_source, "#!/bin/sh\n: >second\n: 2>c\n");
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_partially_overridden_and_heredoc_redirects_unfixed() {
        let source = "#!/bin/sh\n: &>both >stdout\ncat <<EOF <input\nbody\nEOF\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DuplicateRedirect));

        assert_eq!(diagnostics.len(), 4);
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.fix.is_none())
        );
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C051.sh").as_path(),
            &LinterSettings::for_rule(Rule::DuplicateRedirect),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C051_fix_C051.sh", result);
        Ok(())
    }
}
