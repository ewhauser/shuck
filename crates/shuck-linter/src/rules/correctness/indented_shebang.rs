use crate::{Checker, Edit, Fix, FixAvailability, Rule, Violation};

pub struct IndentedShebang;

impl Violation for IndentedShebang {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::IndentedShebang
    }

    fn message(&self) -> String {
        "shebang must start in column 1".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove the leading whitespace before the shebang".to_owned())
    }
}

pub fn indented_shebang(checker: &mut Checker) {
    if let Some((span, indent_span)) = checker
        .facts()
        .indented_shebang_span()
        .zip(checker.facts().indented_shebang_indent_span())
    {
        checker.report_diagnostic_dedup(
            crate::Diagnostic::new(IndentedShebang, span)
                .with_fix(Fix::unsafe_edit(Edit::deletion(indent_span))),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, assert_diagnostics_diff};

    #[test]
    fn reports_indented_shebang_on_first_line() {
        let source = " #!/bin/sh\n:\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::IndentedShebang));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 1);
        assert_eq!(diagnostics[0].span.start.column, 1);
        assert_eq!(diagnostics[0].span.end.column, 1);
    }

    #[test]
    fn reports_indented_shebang_after_header_prelude() {
        let source = "\n #!/bin/sh\n:\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::IndentedShebang));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
        assert_eq!(diagnostics[0].span.end.column, 1);
    }

    #[test]
    fn ignores_non_indented_or_non_header_shebangs() {
        for source in [
            "#!/bin/sh\n:\n",
            "#! /bin/sh\n:\n",
            "\n#!/bin/sh\n:\n",
            "\t# not a shebang\n:\n",
        ] {
            let diagnostics =
                test_snippet(source, &LinterSettings::for_rule(Rule::IndentedShebang));
            assert!(diagnostics.is_empty());
        }
    }

    #[test]
    fn exposes_unsafe_fix_metadata_for_indented_shebangs() {
        let source = "\t#!/bin/sh\n:\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::IndentedShebang));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("remove the leading whitespace before the shebang")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_indented_shebangs() {
        let source = " \t#!/bin/sh\n:\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::IndentedShebang),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/sh\n:\n");
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_non_indented_header_cases_unchanged_when_fixing() {
        for source in ["#!/bin/sh\n:\n", "#! /bin/sh\n:\n", "\n#!/bin/sh\n:\n"] {
            let result = test_snippet_with_fix(
                source,
                &LinterSettings::for_rule(Rule::IndentedShebang),
                Applicability::Unsafe,
            );

            assert_eq!(result.fixes_applied, 0);
            assert_eq!(result.fixed_source, source);
            assert!(result.fixed_diagnostics.is_empty());
        }
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C073.sh").as_path(),
            &LinterSettings::for_rule(Rule::IndentedShebang),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C073_fix_C073.sh", result);
        Ok(())
    }
}
