use shuck_ast::Span;

use crate::{Checker, Edit, Fix, FixAvailability, Rule, Violation};

pub struct PositionalTenBraces;

impl Violation for PositionalTenBraces {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::PositionalTenBraces
    }

    fn message(&self) -> String {
        "use braces for positional parameters above 9".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("wrap the positional parameter in braces".to_owned())
    }
}

pub fn positional_ten_braces(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .positional_parameter_fragments()
        .iter()
        .filter(|fragment| fragment.is_above_nine())
        .map(|fragment| diagnostic_for_positional_parameter(fragment.span(), source))
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic(diagnostic);
    }
}

fn diagnostic_for_positional_parameter(span: Span, source: &str) -> crate::Diagnostic {
    crate::Diagnostic::new(PositionalTenBraces, span).with_fix(Fix::unsafe_edit(Edit::replacement(
        braced_positional_parameter(span, source),
        span,
    )))
}

fn braced_positional_parameter(span: Span, source: &str) -> String {
    let text = span.slice(source);
    let digits = text.strip_prefix('$').unwrap_or(text);
    format!("${{{digits}}}")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_positional_ten_in_assignment_subscripts() {
        let source = "#!/bin/bash\narr[$10]=1\ndeclare other[$10]=1\n";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::PositionalTenBraces));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$10", "$10"]
        );
    }

    #[test]
    fn ignores_special_positional_parameter_expansions() {
        let source = "#!/usr/bin/env bash\nprintf '%s\\n' \"${@:1}\" \"${*:1:2}\"\n";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::PositionalTenBraces));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_single_digit_suffixes_in_nested_quoted_command_substitutions() {
        let source = r#"#!/bin/sh
eval "$(printf '%s\n' x | "$2_rework")"
"#;
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::PositionalTenBraces));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_above_nine_suffixes_in_nested_quoted_command_substitutions() {
        let source = r#"#!/bin/sh
eval "$(printf '%s\n' x | "$10_rework")"
"#;
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::PositionalTenBraces));

        assert_eq!(diagnostics.len(), 1, "diagnostics: {diagnostics:?}");
        assert!(
            diagnostics[0].span.slice(source).contains("$10"),
            "diagnostics: {diagnostics:?}"
        );
    }

    #[test]
    fn attaches_unsafe_fix_metadata() {
        let source = "#!/bin/sh\nprintf '%s\\n' \"$10\"\n";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::PositionalTenBraces));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("wrap the positional parameter in braces")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_unbraced_positional_parameters() {
        let source = "\
#!/bin/sh
printf '%s\\n' \"$10\" $123
arr[$10]=1
printf '%s\\n' \"$2x\"
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::PositionalTenBraces),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
printf '%s\\n' \"${10}\" ${123}
arr[${10}]=1
printf '%s\\n' \"$2x\"
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C025.sh").as_path(),
            &LinterSettings::for_rule(Rule::PositionalTenBraces),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C025_fix_C025.sh", result);
        Ok(())
    }
}
