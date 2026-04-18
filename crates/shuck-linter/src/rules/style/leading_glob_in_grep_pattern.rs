use crate::{Checker, Rule, Violation};

pub struct LeadingGlobInGrepPattern;

impl Violation for LeadingGlobInGrepPattern {
    fn rule() -> Rule {
        Rule::LeadingGlobInGrepPattern
    }

    fn message(&self) -> String {
        "grep patterns should not start with glob-style `*`".to_owned()
    }
}

pub fn leading_glob_in_grep_pattern(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("grep"))
        .filter_map(|fact| fact.options().grep())
        .filter(|grep| !grep.uses_fixed_strings)
        .flat_map(|grep| grep.patterns().iter())
        .filter(|pattern| pattern.source_kind().uses_separate_pattern_word())
        .filter(|pattern| pattern.starts_with_glob_style_star())
        .map(|pattern| pattern.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || LeadingGlobInGrepPattern);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_grep_patterns_that_start_with_glob_style_stars() {
        let source = "\
#!/bin/sh
grep '*MAINTAINER' \"$AUDIT_FILE\"
grep ''*USER \"$AUDIT_FILE\"
grep \"*ENTRYPOINT\" \"$AUDIT_FILE\"
grep \\*CMD2 \"$AUDIT_FILE\"
grep *CMD \"$AUDIT_FILE\"
grep -e '*LABEL' \"$AUDIT_FILE\"
grep --regexp '*EXPOSE' \"$AUDIT_FILE\"
grep -v '^*' \"$AUDIT_FILE\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LeadingGlobInGrepPattern),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "'*MAINTAINER'",
                "''*USER",
                "\"*ENTRYPOINT\"",
                "\\*CMD2",
                "*CMD",
                "'*LABEL'",
                "'*EXPOSE'",
                "'^*'",
            ]
        );
    }

    #[test]
    fn ignores_non_leading_or_unsupported_grep_pattern_forms() {
        let source = "\
#!/bin/sh
grep 'MAINTAINER*' \"$AUDIT_FILE\"
grep '.*MAINTAINER' \"$AUDIT_FILE\"
grep -F '*MAINTAINER' \"$AUDIT_FILE\"
grep -e'*MAINTAINER' \"$AUDIT_FILE\"
grep --regexp='*MAINTAINER' \"$AUDIT_FILE\"
grep '^*$' \"$AUDIT_FILE\"
grep '^*foo' \"$AUDIT_FILE\"
grep \"$pattern\" \"$AUDIT_FILE\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LeadingGlobInGrepPattern),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
