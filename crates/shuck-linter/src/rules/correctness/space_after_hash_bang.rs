use crate::{Checker, Edit, Fix, FixAvailability, Rule, Violation};

pub struct SpaceAfterHashBang;

impl Violation for SpaceAfterHashBang {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::SpaceAfterHashBang
    }

    fn message(&self) -> String {
        "remove the space so the shebang starts with `#!`".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove the whitespace between `#` and `!`".to_owned())
    }
}

pub fn space_after_hash_bang(checker: &mut Checker) {
    if let Some((span, whitespace_span)) = checker
        .facts()
        .space_after_hash_bang_span()
        .zip(checker.facts().space_after_hash_bang_whitespace_span())
    {
        checker.report_diagnostic_dedup(
            crate::Diagnostic::new(SpaceAfterHashBang, span)
                .with_fix(Fix::unsafe_edit(Edit::deletion(whitespace_span))),
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
    fn reports_space_after_hash_bang_on_first_line() {
        let source = "# !/bin/sh\n:\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SpaceAfterHashBang));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 1);
        assert_eq!(diagnostics[0].span.start.column, 2);
        assert_eq!(diagnostics[0].span.end.column, 2);
    }

    #[test]
    fn ignores_valid_or_non_header_comment_lines() {
        for source in [
            "#!/bin/sh\n:\n",
            " #!/bin/sh\n:\n",
            "# comment\n echo ok\n# !/bin/sh\n",
            "echo ok\n# !/bin/sh\n",
        ] {
            let diagnostics =
                test_snippet(source, &LinterSettings::for_rule(Rule::SpaceAfterHashBang));
            assert!(diagnostics.is_empty());
        }
    }

    #[test]
    fn reports_other_whitespace_between_hash_and_bang() {
        let source = "#\t!/bin/sh\n:\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SpaceAfterHashBang));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.column, 2);
        assert_eq!(diagnostics[0].span.end.column, 2);
    }

    #[test]
    fn reports_space_after_hash_bang_after_header_prelude() {
        let source = "\n# !/bin/sh\n:\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SpaceAfterHashBang));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 2);
        assert_eq!(diagnostics[0].span.end.column, 2);
    }

    #[test]
    fn exposes_unsafe_fix_metadata_for_space_after_hash_bang() {
        let source = "# \t!/bin/sh\n:\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SpaceAfterHashBang));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("remove the whitespace between `#` and `!`")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_space_after_hash_bang() {
        let source = "# \t!/bin/sh\n:\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SpaceAfterHashBang),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/sh\n:\n");
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_non_header_cases_unchanged_when_fixing() {
        for source in [
            "#!/bin/sh\n:\n",
            " #!/bin/sh\n:\n",
            "# comment\n echo ok\n# !/bin/sh\n",
            "echo ok\n# !/bin/sh\n",
        ] {
            let result = test_snippet_with_fix(
                source,
                &LinterSettings::for_rule(Rule::SpaceAfterHashBang),
                Applicability::Unsafe,
            );

            assert_eq!(result.fixes_applied, 0);
            assert_eq!(result.fixed_source, source);
            assert!(result.fixed_diagnostics.is_empty());
        }
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_c074_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C074.sh").as_path(),
            &LinterSettings::for_rule(Rule::SpaceAfterHashBang),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C074_fix_C074.sh", result);
        Ok(())
    }
}
