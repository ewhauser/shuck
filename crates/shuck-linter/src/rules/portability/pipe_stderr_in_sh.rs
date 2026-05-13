use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, ShellDialect, Violation};

pub struct PipeStderrInSh;

impl Violation for PipeStderrInSh {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::PipeStderrInSh
    }

    fn message(&self) -> String {
        "the `|&` pipeline operator is not portable in `sh`".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("expand `|&` to `2>&1 |`".to_owned())
    }
}

pub fn pipe_stderr_in_sh(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let diagnostics = checker
        .facts()
        .pipelines()
        .iter()
        .flat_map(|pipeline| pipeline.operators().iter())
        .filter(|operator| operator.op() == shuck_ast::BinaryOp::PipeAll)
        .map(|operator| operator.span())
        .map(|span| {
            Diagnostic::new(PipeStderrInSh, span).with_fix(Fix::safe_edit(Edit::replacement(
                pipe_all_replacement(checker.source(), span),
                span,
            )))
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn pipe_all_replacement(source: &str, span: shuck_ast::Span) -> String {
    let leading = span.start.offset > 0
        && source
            .as_bytes()
            .get(span.start.offset - 1)
            .is_some_and(|byte| !byte.is_ascii_whitespace());
    let trailing = source
        .as_bytes()
        .get(span.end.offset)
        .is_some_and(|byte| !byte.is_ascii_whitespace());

    format!(
        "{}2>&1 |{}",
        if leading { " " } else { "" },
        if trailing { " " } else { "" }
    )
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn reports_pipe_stderr_in_sh() {
        let source = "\
#!/bin/sh
echo test |& grep -q foo
echo first |& grep -q foo | cat
echo left | grep -q foo |& cat
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::PipeStderrInSh));

        assert_eq!(diagnostics.len(), 3);
        assert_eq!(diagnostics[0].span.slice(source), "|&");
        assert_eq!(diagnostics[1].span.slice(source), "|&");
        assert_eq!(diagnostics[2].span.slice(source), "|&");
    }

    #[test]
    fn ignores_pipe_stderr_in_bash() {
        let source = "\
#!/bin/bash
echo test |& grep -q foo
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::PipeStderrInSh));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_safe_fix_to_pipe_stderr_operator() {
        let source = "#!/bin/sh\necho test |& grep -q foo\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::PipeStderrInSh),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\necho test 2>&1 | grep -q foo\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn applies_safe_fix_to_compact_pipe_stderr_operator() {
        let source = "\
#!/bin/sh
first|&next
left |&right
left|& right
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::PipeStderrInSh),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
first 2>&1 | next
left 2>&1 | right
left 2>&1 | right
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }
}
