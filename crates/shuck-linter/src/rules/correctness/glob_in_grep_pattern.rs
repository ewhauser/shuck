use crate::{Checker, Edit, Fix, FixAvailability, Rule, Violation};

pub struct GlobInGrepPattern;

impl Violation for GlobInGrepPattern {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::GlobInGrepPattern
    }

    fn message(&self) -> String {
        "use regex-style wildcards in grep patterns, not glob-style `*`".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("replace glob-style `*` with `.*` in the pattern".to_owned())
    }
}

pub fn glob_in_grep_pattern(checker: &mut Checker) {
    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("grep"))
        .filter_map(|fact| fact.options().grep())
        .filter(|grep| !grep.uses_fixed_strings)
        .flat_map(|grep| grep.patterns().iter())
        .filter(|pattern| !pattern.starts_with_glob_style_star())
        .filter(|pattern| pattern.has_glob_style_star_confusion())
        .map(|pattern| {
            let span = pattern.span();
            let fix = Fix::unsafe_edits(
                pattern
                    .glob_style_star_replacement_spans()
                    .iter()
                    .copied()
                    .map(|span| Edit::replacement(".*", span)),
            );
            crate::Diagnostic::new(GlobInGrepPattern, span).with_fix(fix)
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_glob_style_stars_in_grep_patterns() {
        let source = "\
#!/bin/sh
grep start* out.txt
grep \"start*\" out.txt
grep 'foo*bar' out.txt
grep 'foo\\*bar*' out.txt
grep foo*bar out.txt
grep -efoo* out.txt
grep --regexp start* out.txt
grep --regexp='start*' out.txt
grep --regexp=foo*bar out.txt
grep --context 3 foo*bar out.txt
grep --exclude '*.txt' foo*bar out.txt
grep --label stdin foo*bar out.txt
grep \"foo*bar\" out.txt
grep item\\* out.txt
grep -E \"foo*bar\" out.txt
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GlobInGrepPattern));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "start*",
                "\"start*\"",
                "'foo*bar'",
                "'foo\\*bar*'",
                "foo*bar",
                "-efoo*",
                "start*",
                "--regexp='start*'",
                "--regexp=foo*bar",
                "foo*bar",
                "foo*bar",
                "foo*bar",
                "\"foo*bar\"",
                "item\\*",
                "\"foo*bar\"",
            ]
        );
    }

    #[test]
    fn ignores_regex_operators_and_non_pattern_operands() {
        let source = "\
#!/bin/sh
grep \"a.*\" out.txt
grep a.* out.txt
grep \"[ab]*\" out.txt
grep [ab]* out.txt
grep '*start' out.txt
grep '*start*' out.txt
grep -e'*start' out.txt
grep --regexp='*start' out.txt
grep item\\\\* out.txt
grep '^ *#' out.txt
grep '\"name\": *\"$x\"' out.txt
grep '^#* OPTIONS #*$' out.txt
grep -Eo 'https?://[[:alnum:]./?&!$#%@*;:+~_=-]+' out.txt
grep '^root:[:!*]' out.txt
grep -e 'Swarm:*\\sactive\\s*' out.txt
grep 'foo*bar+' out.txt
grep '^foo*bar$' out.txt
grep -F foo*bar out.txt
grep -F \"foo*bar\" out.txt
grep --fixed-strings foo*bar out.txt
grep --fixed-strings \"foo*bar\" out.txt
grep -eo foo* out.txt
grep -efoo out.txt
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GlobInGrepPattern));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn attaches_unsafe_fix_metadata_for_grep_patterns() {
        let source = "#!/bin/sh\ngrep start* out.txt\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GlobInGrepPattern));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("replace glob-style `*` with `.*` in the pattern")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_glob_style_grep_patterns() {
        let source = "\
#!/bin/sh
grep start* out.txt
grep 'foo\\*bar*' out.txt
grep item\\* out.txt
grep --regexp='start*' out.txt
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::GlobInGrepPattern),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 4);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
grep start.* out.txt
grep 'foo\\*bar.*' out.txt
grep item.* out.txt
grep --regexp='start.*' out.txt
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_non_violating_grep_patterns_unchanged_when_fixing() {
        let source = "\
#!/bin/sh
grep '*start' out.txt
grep 'a.*' out.txt
grep -F foo*bar out.txt
grep --fixed-strings \"foo*bar\" out.txt
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::GlobInGrepPattern),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C080_fix_C080.sh").as_path(),
            &LinterSettings::for_rule(Rule::GlobInGrepPattern),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C080_fix_C080.sh", result);
        Ok(())
    }
}
