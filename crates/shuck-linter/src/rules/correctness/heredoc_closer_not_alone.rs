use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct HeredocCloserNotAlone;

impl Violation for HeredocCloserNotAlone {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::HeredocCloserNotAlone
    }

    fn message(&self) -> String {
        "this here-document closer must be on its own line".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("append the here-document closer".to_owned())
    }
}

pub fn heredoc_closer_not_alone(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .heredoc_closer_not_alone_spans()
        .iter()
        .copied()
        .map(|span| {
            Diagnostic::new(HeredocCloserNotAlone, span)
                .with_fix(append_heredoc_closer_fix(source, span.slice(source)))
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn append_heredoc_closer_fix(source: &str, delimiter: &str) -> Fix {
    let content = if source.is_empty() || source.ends_with('\n') {
        format!("{delimiter}\n")
    } else {
        format!("\n{delimiter}\n")
    };

    Fix::safe_edit(Edit::insertion(source.len(), content))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_content_prefixed_terminator_lines() {
        let source = "\
#!/bin/sh
cat <<EOF
x EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::HeredocCloserNotAlone),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
        assert_eq!(diagnostics[0].span.slice(source), "EOF");
    }

    #[test]
    fn reports_content_prefixed_terminators_for_tab_stripped_heredocs() {
        let source = "\
#!/bin/sh
cat <<-EOF
\tx EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::HeredocCloserNotAlone),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
        assert_eq!(diagnostics[0].span.slice(source), "EOF");
    }

    #[test]
    fn ignores_properly_closed_heredocs() {
        let source = "\
#!/bin/sh
cat <<EOF
x
EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::HeredocCloserNotAlone),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_safe_fix_by_appending_heredoc_closer() {
        let source = "\
#!/bin/sh
cat <<EOF
x EOF";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::HeredocCloserNotAlone),
            Applicability::Safe,
        );

        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
cat <<EOF
x EOF
EOF
"
        );
        assert_eq!(result.fixes_applied, 1);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_safe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C144.sh").as_path(),
            &LinterSettings::for_rule(Rule::HeredocCloserNotAlone),
            Applicability::Safe,
        )?;

        assert_diagnostics_diff!("C144_fix_C144.sh", result);
        Ok(())
    }
}
