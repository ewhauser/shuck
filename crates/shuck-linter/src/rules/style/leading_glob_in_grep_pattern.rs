use crate::{Checker, GrepPatternSourceKind, Rule, Violation};

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
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("grep"))
        .filter_map(|fact| fact.options().grep())
        .filter(|grep| !grep.uses_fixed_strings)
        .flat_map(|grep| grep.patterns().iter())
        .filter(|pattern| pattern.source_kind().uses_separate_pattern_word())
        .filter(|pattern| {
            pattern
                .static_text()
                .is_some_and(|text| text.starts_with('*') || text == "^*")
        })
        .filter(|pattern| {
            word_text_starts_with_glob_star(pattern.span().slice(source), pattern.source_kind())
        })
        .map(|pattern| pattern.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || LeadingGlobInGrepPattern);
}

fn word_text_starts_with_glob_star(text: &str, source_kind: GrepPatternSourceKind) -> bool {
    let bytes = text.as_bytes();
    match source_kind {
        GrepPatternSourceKind::ImplicitOperand
        | GrepPatternSourceKind::ShortOptionSeparate
        | GrepPatternSourceKind::LongOptionSeparate => {
            matches!(
                bytes,
                [b'*', ..]
                    | [b'"', b'*', ..]
                    | [b'\'', b'*', ..]
                    | [b'^', b'*']
                    | [b'"', b'^', b'*', b'"']
                    | [b'\'', b'^', b'*', b'\'']
            )
        }
        GrepPatternSourceKind::ShortOptionAttached | GrepPatternSourceKind::LongOptionAttached => {
            false
        }
    }
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
grep \"*ENTRYPOINT\" \"$AUDIT_FILE\"
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
                "\"*ENTRYPOINT\"",
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
grep \\*MAINTAINER \"$AUDIT_FILE\"
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
