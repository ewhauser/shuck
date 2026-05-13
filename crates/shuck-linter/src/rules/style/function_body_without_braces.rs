use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, ShellDialect, Violation};

pub struct FunctionBodyWithoutBraces;

impl Violation for FunctionBodyWithoutBraces {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::FunctionBodyWithoutBraces
    }

    fn message(&self) -> String {
        "function body should use a brace group instead of a bare compound command".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("wrap the function body in braces".to_owned())
    }
}

pub fn function_body_without_braces(checker: &mut Checker) {
    if !matches!(
        checker.shell(),
        ShellDialect::Sh | ShellDialect::Bash | ShellDialect::Dash | ShellDialect::Ksh
    ) {
        return;
    }

    let source = checker.source();
    let diagnostics = checker
        .facts()
        .command_facts()
        .function_body_without_braces_spans()
        .iter()
        .copied()
        .map(|span| {
            Diagnostic::new(FunctionBodyWithoutBraces, span)
                .with_fix(wrap_function_body_fix(span, source))
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn wrap_function_body_fix(span: Span, source: &str) -> Fix {
    let insert_end = trim_trailing_whitespace_offset(span, source);
    Fix::safe_edits([
        Edit::insertion(span.start.offset, "{ "),
        Edit::insertion(insert_end, "; }"),
    ])
}

fn trim_trailing_whitespace_offset(span: Span, source: &str) -> usize {
    let mut offset = span.end.offset;
    while offset > span.start.offset {
        let Some(ch) = source[..offset].chars().next_back() else {
            break;
        };
        if !ch.is_whitespace() {
            break;
        }
        offset -= ch.len_utf8();
    }
    offset
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_function_bodies_written_as_bare_compound_commands() {
        let source = "\
#!/bin/bash
f() [[ -n \"$x\" ]]
g() if true; then :; fi
h() case x in x) :;; esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionBodyWithoutBraces),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "[[ -n \"$x\" ]]",
                "if true; then :; fi\n",
                "case x in x) :;; esac\n",
            ]
        );
    }

    #[test]
    fn ignores_function_bodies_shellcheck_accepts() {
        let source = "\
#!/bin/bash
f() { [[ -n \"$x\" ]]; }
g() { ( echo hi ); }
h() ( echo hi )
i() (( x++ ))
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionBodyWithoutBraces),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn applies_safe_fix_by_wrapping_function_body_in_braces() {
        let source = "\
#!/bin/bash
f() [[ -n \"$x\" ]]
g() if true; then :; fi
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::FunctionBodyWithoutBraces),
            Applicability::Safe,
        );

        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
f() { [[ -n \"$x\" ]]; }
g() { if true; then :; fi; }
"
        );
        assert_eq!(result.fixes_applied, 2);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_safe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("style").join("S041.sh").as_path(),
            &LinterSettings::for_rule(Rule::FunctionBodyWithoutBraces),
            Applicability::Safe,
        )?;

        assert_diagnostics_diff!("S041_fix_S041.sh", result);
        Ok(())
    }
}
