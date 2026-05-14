use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct AssignmentSpacing;

impl Violation for AssignmentSpacing {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::AssignmentSpacing
    }

    fn message(&self) -> String {
        "a space after `=` keeps this from being one assignment word".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("delete the whitespace after `=`".to_owned())
    }
}

pub fn assignment_spacing(checker: &mut Checker) {
    let spans = checker
        .facts()
        .command_facts()
        .assignment_spacing_spans()
        .to_vec();
    for span in spans {
        checker.report_diagnostic_dedup(
            Diagnostic::new(AssignmentSpacing, span)
                .with_fix(Fix::unsafe_edit(Edit::deletion(span))),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect, assert_diagnostics_diff};

    #[test]
    fn reports_spaces_after_empty_assignments() {
        let source = "\
#!/bin/sh
foo= bar
foo+= append
A= B= cmd
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::AssignmentSpacing));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![" ", " ", " ", " "]
        );
    }

    #[test]
    fn reports_declaration_operands_independently() {
        let source = "\
#!/bin/sh
export foo= bar baz= qux
readonly pinned= value
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::AssignmentSpacing));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![" ", " ", " "]
        );
    }

    #[test]
    fn reports_line_continued_assignment_gaps() {
        let source = "\
#!/bin/sh
ARCH= \\
EARCH= \\
./configure
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::AssignmentSpacing));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![" \\\n", " \\\n"]
        );
    }

    #[test]
    fn ignores_ifs_empty_assignments() {
        let source = "\
#!/bin/sh
while IFS= read -r line; do
  printf '%s\\n' \"$line\"
done
local IFS= value
IFS=  read -r first_line
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::AssignmentSpacing));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_complete_assignments_and_later_arguments() {
        let source = "\
#!/bin/sh
foo=bar
foo=
foo =bar
echo foo= bar
foo=bar cmd baz= qux
export foo=
comment='name= value'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::AssignmentSpacing));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_zsh_scripts() {
        let source = "#!/bin/zsh\nfoo= bar\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AssignmentSpacing).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![" "]
        );
    }

    #[test]
    fn reports_after_comment_apostrophe() {
        let source = "\
#!/bin/sh
# it's okay to use contractions in comments
foo= bar
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::AssignmentSpacing));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![" "]
        );
    }

    #[test]
    fn attaches_unsafe_fix_metadata() {
        let source = "#!/bin/sh\nfoo= bar\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::AssignmentSpacing));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("delete the whitespace after `=`")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_assignment_spacing() {
        let source = "\
#!/bin/sh
foo= bar
foo+= append
export left= right other= value
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::AssignmentSpacing),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 4);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\nfoo=bar\nfoo+=append\nexport left=right other=value\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C024.sh").as_path(),
            &LinterSettings::for_rule(Rule::AssignmentSpacing),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C024_fix_C024.sh", result);
        Ok(())
    }
}
