use crate::{Checker, Rule, Violation, word_unquoted_glob_pattern_spans};

pub struct GlobInGrepPattern;

impl Violation for GlobInGrepPattern {
    fn rule() -> Rule {
        Rule::GlobInGrepPattern
    }

    fn message(&self) -> String {
        "quote grep patterns so the shell does not expand them first".to_owned()
    }
}

pub fn glob_in_grep_pattern(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("grep"))
        .filter_map(|fact| fact.options().grep())
        .flat_map(|grep| grep.pattern_words().iter().copied())
        .filter(|word| !word_unquoted_glob_pattern_spans(word, checker.source()).is_empty())
        .map(|word| word.span)
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || GlobInGrepPattern);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_unquoted_globs_in_grep_patterns() {
        let source = "\
#!/bin/sh
grep start* out.txt
grep -e item? out.txt
grep --regexp item,[0-4] out.txt
grep -Eq item,[0-4] out.txt
grep -eo item* out.txt
grep -F -- item,[0-4] out.txt
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GlobInGrepPattern));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "start*",
                "item?",
                "item,[0-4]",
                "item,[0-4]",
                "item*",
                "item,[0-4]"
            ]
        );
    }

    #[test]
    fn ignores_quoted_patterns_and_non_pattern_operands() {
        let source = "\
#!/bin/sh
grep \"start*\" out.txt
grep --regexp='item,[0-4]' out.txt
grep --regexp=item,[0-4] out.txt
grep -eitem* out.txt
grep -f patterns.txt item,[0-4] out.txt
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GlobInGrepPattern));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_grep_patterns_inside_command_substitution() {
        let source = "\
#!/bin/sh
checksum=\"$(grep -Ehrow [0-9a-f]{40} ${template}|sort|uniq|tr '\\n' ' ')\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GlobInGrepPattern));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["[0-9a-f]{40}"]
        );
    }
}
