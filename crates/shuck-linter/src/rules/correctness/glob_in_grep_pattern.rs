use crate::{Checker, Rule, Violation};

pub struct GlobInGrepPattern;

impl Violation for GlobInGrepPattern {
    fn rule() -> Rule {
        Rule::GlobInGrepPattern
    }

    fn message(&self) -> String {
        "use regex-style wildcards in grep patterns, not glob-style `*`".to_owned()
    }
}

pub fn glob_in_grep_pattern(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("grep"))
        .filter_map(|fact| fact.options().grep())
        .filter(|grep| !grep.uses_fixed_strings)
        .flat_map(|grep| grep.patterns().iter())
        .filter(|pattern| !pattern.starts_with_glob_style_star())
        .filter(|pattern| pattern.has_glob_style_star_confusion())
        .map(|pattern| pattern.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || GlobInGrepPattern);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_glob_style_stars_in_grep_patterns() {
        let source = "\
#!/bin/sh
grep start* out.txt
grep \"start*\" out.txt
grep 'foo*bar' out.txt
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
}
