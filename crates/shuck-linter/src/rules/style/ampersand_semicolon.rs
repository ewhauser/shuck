use crate::{Checker, Edit, Fix, FixAvailability, Rule, Violation};

pub struct AmpersandSemicolon;

impl Violation for AmpersandSemicolon {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::AmpersandSemicolon
    }

    fn message(&self) -> String {
        "background command should not be followed by `;`".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove the stray `;` after `&`".to_owned())
    }
}

pub fn ampersand_semicolon(checker: &mut Checker) {
    let spans = checker.facts().background_semicolon_spans().to_vec();
    for span in spans {
        checker.report_diagnostic_dedup(
            crate::Diagnostic::new(AmpersandSemicolon, span)
                .with_fix(Fix::safe_edit(Edit::deletion(span))),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_background_followed_by_semicolon() {
        let source = "#!/bin/sh\necho x &;\necho y & ;\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::AmpersandSemicolon));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), ";");
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[1].span.slice(source), ";");
        assert_eq!(diagnostics[1].span.start.line, 3);
    }

    #[test]
    fn ignores_background_without_semicolon() {
        let source = "#!/bin/sh\necho x &\nwait\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::AmpersandSemicolon));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_safe_fix_to_background_semicolons() {
        let source = "#!/bin/sh\necho x &;\necho y & ;\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::AmpersandSemicolon),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(result.fixed_source, "#!/bin/sh\necho x &\necho y & \n");
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn ignores_case_item_terminators_after_background() {
        let source = "\
#!/bin/bash
case ${1-} in
  break) printf '%s\\n' ok &;;
  spaced) printf '%s\\n' ok & ;;
  fallthrough) printf '%s\\n' ok & ;&
  continue) printf '%s\\n' ok & ;;&
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::AmpersandSemicolon));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn leaves_case_item_terminators_unchanged_when_fixing() {
        let source = "\
#!/bin/bash
case ${1-} in
  break) printf '%s\\n' ok &;;
  spaced) printf '%s\\n' ok & ;;
  fallthrough) printf '%s\\n' ok & ;&
  continue) printf '%s\\n' ok & ;;&
esac
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::AmpersandSemicolon),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_safe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("style").join("S074.sh").as_path(),
            &LinterSettings::for_rule(Rule::AmpersandSemicolon),
            Applicability::Safe,
        )?;

        assert_diagnostics_diff!("S074_fix_S074.sh", result);
        Ok(())
    }
}
