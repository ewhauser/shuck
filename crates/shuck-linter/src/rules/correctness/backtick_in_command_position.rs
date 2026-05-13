use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct BacktickInCommandPosition;

impl Violation for BacktickInCommandPosition {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::BacktickInCommandPosition
    }

    fn message(&self) -> String {
        "run the command directly instead of executing backtick output as a command name".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove the command-name backticks".to_owned())
    }
}

pub fn backtick_in_command_position(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .command_facts()
        .backtick_command_name_spans()
        .to_vec();

    for span in spans {
        checker.report_diagnostic_dedup(Diagnostic::new(BacktickInCommandPosition, span).with_fix(
            Fix::unsafe_edit(Edit::replacement(
                backtick_inner_text(span.slice(source)),
                span,
            )),
        ));
    }
}

fn backtick_inner_text(raw: &str) -> String {
    raw.strip_prefix('`')
        .and_then(|inner| inner.strip_suffix('`'))
        .unwrap_or(raw)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_plain_backticks_used_as_command_names() {
        let source = "\
#!/bin/sh
`echo hello` | cat
if `echo true`; then :; fi
FOO=1 `echo run`
`echo run` 2>/dev/null
true && `echo go` 2>/dev/null
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BacktickInCommandPosition),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "`echo hello`",
                "`echo true`",
                "`echo run`",
                "`echo run`",
                "`echo go`",
            ]
        );
    }

    #[test]
    fn ignores_wrapped_quoted_affixed_and_argument_backticks() {
        let source = "\
#!/bin/sh
command `echo hello`
\"`echo hello`\" | cat
x`echo hello`
echo `date`
`echo hello` arg
true && `echo hello` arg
`echo hello` arg 2>/dev/null
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BacktickInCommandPosition),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_backtick_command_names() {
        let source = "\
#!/bin/sh
`echo hello` | cat
if `echo true`; then :; fi
FOO=1 `echo run`
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::BacktickInCommandPosition),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
echo hello | cat
if echo true; then :; fi
FOO=1 echo run
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_argument_backticks_unchanged_when_fixing() {
        let source = "\
#!/bin/sh
command `echo hello`
echo `date`
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::BacktickInCommandPosition),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C093.sh").as_path(),
            &LinterSettings::for_rule(Rule::BacktickInCommandPosition),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C093_fix_C093.sh", result);
        Ok(())
    }
}
