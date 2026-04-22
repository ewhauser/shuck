use shuck_ast::Span;

use crate::{Checker, Edit, Fix, FixAvailability, Rule, ShellDialect, Violation};

const FIX_TITLE: &str = "remove the leading `$` from the quoted string";

pub struct DollarStringInSh;

impl Violation for DollarStringInSh {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::DollarStringInSh
    }

    fn message(&self) -> String {
        "`$\"...\"` strings are not portable in `sh`".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some(FIX_TITLE.to_owned())
    }
}

pub fn dollar_string_in_sh(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .dollar_double_quoted_fragments()
        .iter()
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();

    for span in spans {
        checker.report_diagnostic_dedup(
            crate::Diagnostic::new(DollarStringInSh, span).with_fix(dollar_string_in_sh_fix(span)),
        );
    }
}

fn dollar_string_in_sh_fix(span: Span) -> Fix {
    Fix::unsafe_edit(Edit::deletion_at(
        span.start.offset,
        span.start.offset + "$".len(),
    ))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::FIX_TITLE;
    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect, assert_diagnostics_diff};

    #[test]
    fn anchors_on_each_dollar_double_quoted_fragment() {
        let source = "\
#!/bin/sh
echo $\"Usage: $0 {start|stop}\"
printf '%s\\n' \"$\"'not-a-dollar-double-quote'\" plain
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarStringInSh));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$\"Usage: $0 {start|stop}\""]
        );
    }

    #[test]
    fn ignores_dollar_double_quoted_fragments_in_bash() {
        let source = "echo $\"hi\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::DollarStringInSh).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn attaches_unsafe_fix_metadata_to_reported_fragments() {
        let source = "#!/bin/sh\necho $\"hi\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarStringInSh));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(diagnostics[0].fix_title.as_deref(), Some(FIX_TITLE));
    }

    #[test]
    fn applies_unsafe_fix_to_dollar_double_quoted_fragments() {
        let source = "\
#!/bin/sh
echo $\"Usage: $0 {start|stop}\"
printf '%s\\n' prefix$\"translated\"suffix
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::DollarStringInSh),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
echo \"Usage: $0 {start|stop}\"
printf '%s\\n' prefix\"translated\"suffix
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_portable_double_quoted_strings_unchanged_when_fixing() {
        let source = "\
#!/bin/sh
echo \"Usage: $0 {start|stop}\"
printf '%s\\n' prefix\"translated\"suffix
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::DollarStringInSh),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("portability").join("X055.sh").as_path(),
            &LinterSettings::for_rule(Rule::DollarStringInSh),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("X055_fix_X055.sh", result);
        Ok(())
    }
}
