use crate::{Checker, Rule, Violation, word_unquoted_glob_pattern_spans};

pub struct UnquotedGrepRegex;

impl Violation for UnquotedGrepRegex {
    fn rule() -> Rule {
        Rule::UnquotedGrepRegex
    }

    fn message(&self) -> String {
        "quote grep regex patterns so the shell does not expand them first".to_owned()
    }
}

pub fn unquoted_grep_regex(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("grep"))
        .filter_map(|fact| fact.options().grep())
        .flat_map(|grep| grep.patterns().iter())
        .filter(|pattern| {
            !word_unquoted_glob_pattern_spans(pattern.word(), checker.source()).is_empty()
        })
        .map(|pattern| pattern.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || UnquotedGrepRegex);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_unquoted_grep_patterns_that_can_glob_expand() {
        let source = "\
#!/bin/sh
grep start* out.txt
grep -e item? out.txt
grep -eitem* out.txt
grep -oe item* out.txt
grep --regexp item,[0-4] out.txt
grep -Eq item,[0-4] out.txt
grep --regexp=foo*bar out.txt
grep --context 3 foo*bar out.txt
grep --exclude '*.txt' foo*bar out.txt
grep --label stdin item? out.txt
grep -F -- item,[0-4] out.txt
grep -F foo*bar out.txt
grep [0-9a-f]{40} out.txt
checksum=\"$(grep -Ehrow [0-9a-f]{40} ${template}|sort|uniq|tr '\\n' ' ')\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedGrepRegex));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "start*",
                "item?",
                "-eitem*",
                "item*",
                "item,[0-4]",
                "item,[0-4]",
                "--regexp=foo*bar",
                "foo*bar",
                "foo*bar",
                "item?",
                "item,[0-4]",
                "foo*bar",
                "[0-9a-f]{40}",
                "[0-9a-f]{40}"
            ]
        );
    }

    #[test]
    fn ignores_quoted_patterns_and_non_pattern_operands() {
        let source = "\
#!/bin/sh
grep \"start*\" out.txt
grep --regexp='item,[0-4]' out.txt
grep -eo item* out.txt
grep -f patterns.txt item,[0-4] out.txt
grep --exclude '*.txt' \"foo*bar\" out.txt
grep \\[ab\\]\\* out.txt
grep -F \"foo*bar\" out.txt
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedGrepRegex));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_nested_grep_patterns_with_split_literal_bracket_globs() {
        let source = "\
#!/bin/sh
for file in $(ls /tmp | grep -v [/$] | grep -v ' '); do
    :
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedGrepRegex));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["[/$]"]
        );
    }
}
