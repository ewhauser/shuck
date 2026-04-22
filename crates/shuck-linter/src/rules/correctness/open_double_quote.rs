use crate::{Checker, Edit, Fix, FixAvailability, Rule, Violation};

pub struct OpenDoubleQuote;

impl Violation for OpenDoubleQuote {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::OpenDoubleQuote
    }

    fn message(&self) -> String {
        "quoted string looks unterminated".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("rewrite the word as one double-quoted string".to_owned())
    }
}

pub fn open_double_quote(checker: &mut Checker) {
    let diagnostics = checker
        .facts()
        .open_double_quote_fragments()
        .iter()
        .map(|fragment| {
            crate::Diagnostic::new(OpenDoubleQuote, fragment.span()).with_fix(Fix::unsafe_edit(
                Edit::replacement(
                    fragment.replacement().to_owned(),
                    fragment.replacement_span(),
                ),
            ))
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_suspicious_opening_double_quote() {
        let source = "#!/bin/bash\necho \"#!/bin/bash\nif [[ \"$@\" =~ x ]]; then :; fi\n\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::OpenDoubleQuote));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 6);
        assert_eq!(diagnostics[0].span.start, diagnostics[0].span.end);
    }

    #[test]
    fn ignores_regular_multiline_double_quotes() {
        let source = "#!/bin/sh\necho \"line one\nline two\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::OpenDoubleQuote));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_sc2140_style_multiline_quote_joins() {
        let source = "\
#!/bin/bash
echo \"[Unit]
Description=Heimdall
ExecStart=\"/usr/bin/php\" artisan serve
WantedBy=multi-user.target\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::OpenDoubleQuote));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_multiline_quote_with_suffix_expansion() {
        let source = "\
#!/bin/bash
echo \"line one
line two\"$suffix
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::OpenDoubleQuote));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 6);
        assert_eq!(diagnostics[0].span.start, diagnostics[0].span.end);
    }

    #[test]
    fn reports_reopened_single_quoted_windows() {
        let source = "\
#!/bin/sh
echo 'line one
line two''tail'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::OpenDoubleQuote));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 6);
        assert_eq!(diagnostics[0].span.start, diagnostics[0].span.end);
    }

    #[test]
    fn reports_each_reopened_quote_window_in_a_single_word() {
        let source = "\
#!/bin/bash
echo \"alpha
\"$foo\"beta
\"$bar\"gamma\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::OpenDoubleQuote));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 6);
        assert_eq!(diagnostics[1].span.start.line, 3);
        assert_eq!(diagnostics[1].span.start.column, 6);
    }

    #[test]
    fn reports_independent_reopened_quote_windows_across_multiple_arguments() {
        let source = "\
#!/bin/bash
echo \"a
\"$x\"b\" \"c
\"$y\"d\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::OpenDoubleQuote));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 6);
        assert_eq!(diagnostics[1].span.start.line, 3);
        assert_eq!(diagnostics[1].span.start.column, 8);
    }

    #[test]
    fn reports_independent_reopened_quote_windows_with_prefixed_later_arguments() {
        let source = "\
#!/bin/bash
echo \"a
\"$x\"b\" pre\"c
\"$y\"d\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::OpenDoubleQuote));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 6);
        assert_eq!(diagnostics[1].span.start.line, 3);
        assert_eq!(diagnostics[1].span.start.column, 11);
    }

    #[test]
    fn attaches_unsafe_fix_metadata() {
        let source = "#!/bin/bash\necho \"#!/bin/bash\nif [[ \"$@\" =~ x ]]; then :; fi\n\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::OpenDoubleQuote));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("rewrite the word as one double-quoted string")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_scalar_gap_words() {
        let source = "\
#!/bin/bash
echo \"alpha
\"$foo\"beta
\"$bar\"gamma\"
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::OpenDoubleQuote),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
echo \"alpha
${foo}beta
${bar}gamma\"
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_literal_gap_words() {
        let source = "\
#!/bin/sh
echo \"help text
say \"configure\" now
\"
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::OpenDoubleQuote),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
echo \"help text
say configure now
\"
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C039.sh").as_path(),
            &LinterSettings::for_rule(Rule::OpenDoubleQuote),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C039_fix_C039.sh", result);
        Ok(())
    }

    #[test]
    fn reports_multiline_triple_quote_script_builders() {
        let source = "\
#!/bin/bash
echo \"\"\"#!/usr/bin/env bash
echo \"GEM_HOME FIRST: \\$GEM_HOME\"
echo \"GEM_PATH: \\$GEM_PATH\"
\"\"\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::OpenDoubleQuote));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 8);
        assert_eq!(diagnostics[0].span.start, diagnostics[0].span.end);
    }
}
