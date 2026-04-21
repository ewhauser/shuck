use crate::{FixAvailability, Rule, Violation};

pub struct MissingFi;

impl Violation for MissingFi {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::MissingFi
    }

    fn message(&self) -> String {
        "this `if` block is missing a closing `fi`".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("append a closing `fi`".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_missing_fi_from_parse_diagnostics() {
        let source = "#!/bin/sh\nif true; then\n  :\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MissingFi));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code(), "C035");
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn attaches_unsafe_fix_metadata() {
        let source = "#!/bin/sh\nif true; then\n  :\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MissingFi));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("append a closing `fi`")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_unclosed_if_blocks() {
        let source = "#!/bin/sh\nif true; then\n  :\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::MissingFi),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/sh\nif true; then\n  :\nfi\n");
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn inserts_a_newline_before_fi_when_source_lacks_one() {
        let source = "#!/bin/sh\nif true; then\n  :";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::MissingFi),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/sh\nif true; then\n  :\nfi\n");
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C035.sh").as_path(),
            &LinterSettings::for_rule(Rule::MissingFi),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C035_fix_C035.sh", result);
        Ok(())
    }
}
