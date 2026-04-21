use crate::{
    Checker, Edit, Fix, FixAvailability, FunctionHeaderFact, Rule, ShellDialect, Violation,
};

pub struct FunctionKeywordInSh;

impl Violation for FunctionKeywordInSh {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::FunctionKeywordInSh
    }

    fn message(&self) -> String {
        "`function` with trailing `()` is not portable in `sh` scripts".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove the leading `function` keyword".to_owned())
    }
}

pub fn function_keyword_in_sh(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let diagnostics = checker
        .facts()
        .function_headers()
        .iter()
        .filter(|header| header.uses_function_keyword() && header.has_trailing_parens())
        .map(|header| {
            let span = header.function_span_in_source(checker.source());
            let diagnostic = crate::Diagnostic::new(FunctionKeywordInSh, span);
            match function_keyword_in_sh_fix(header) {
                Some(fix) => diagnostic.with_fix(fix),
                None => diagnostic,
            }
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn function_keyword_in_sh_fix(header: &FunctionHeaderFact<'_>) -> Option<Fix> {
    let keyword_span = header.function_keyword_span()?;
    let (_, name_span) = header.static_name_entry()?;

    Some(Fix::safe_edit(Edit::deletion_at(
        keyword_span.start.offset,
        name_span.start.offset,
    )))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect, assert_diagnostics_diff};

    #[test]
    fn reports_function_keyword_with_parens_over_full_function_span() {
        let source = "\
#!/bin/sh
function greet()
{
  printf '%s\\n' hi
}
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::FunctionKeywordInSh));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "function greet()\n{\n  printf '%s\\n' hi\n}"
        );
    }

    #[test]
    fn ignores_function_keyword_without_trailing_parens() {
        let source = "\
#!/bin/sh
function greet { :; }
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::FunctionKeywordInSh));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_non_sh_shells() {
        let source = "\
#!/bin/bash
function greet() { :; }
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionKeywordInSh).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn applies_safe_fix_to_function_keyword_with_parens() {
        let source = "\
#!/bin/sh
function greet()
{
  printf '%s\\n' hi
}
function   spaced() { :; }
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::FunctionKeywordInSh),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\ngreet()\n{\n  printf '%s\\n' hi\n}\nspaced() { :; }\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_function_keyword_without_parens_unchanged_when_fixing() {
        let source = "\
#!/bin/sh
function greet { :; }
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::FunctionKeywordInSh),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn reports_dynamic_function_names_without_panicking_or_fixing() {
        let source = "\
#!/bin/sh
function $0_error() { :; }
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::FunctionKeywordInSh),
            Applicability::Safe,
        );

        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixed_diagnostics.len(), 1);
        assert_eq!(
            result.fixed_diagnostics[0].span.slice(source),
            "function $0_error() { :; }"
        );
    }

    #[test]
    fn snapshots_safe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("portability").join("X052.sh").as_path(),
            &LinterSettings::for_rule(Rule::FunctionKeywordInSh),
            Applicability::Safe,
        )?;

        assert_diagnostics_diff!("X052_fix_X052.sh", result);
        Ok(())
    }
}
