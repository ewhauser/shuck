use crate::{Checker, Edit, Fix, FixAvailability, Rule, Violation};

pub struct UnquotedGrepRegex;

impl Violation for UnquotedGrepRegex {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::UnquotedGrepRegex
    }

    fn message(&self) -> String {
        "quote grep regex patterns so the shell does not expand them first".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("quote the reported grep pattern word".to_owned())
    }
}

pub fn unquoted_grep_regex(checker: &mut Checker) {
    let source = checker.source();
    let facts = checker.facts();
    let diagnostics = facts
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("grep"))
        .filter_map(|fact| fact.options().grep())
        .flat_map(|grep| grep.patterns().iter())
        .filter(|pattern| {
            facts
                .any_word_fact(pattern.word().span)
                .is_some_and(|fact| !fact.active_literal_glob_spans(source).is_empty())
        })
        .map(|pattern| {
            let span = pattern.span();
            let replacement = facts
                .any_word_fact(span)
                .expect("grep pattern span should map to a word fact")
                .single_double_quoted_replacement(source);
            crate::Diagnostic::new(UnquotedGrepRegex, span)
                .with_fix(Fix::unsafe_edit(Edit::replacement(replacement, span)))
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
    fn attaches_unsafe_fix_metadata_for_reported_grep_patterns() {
        let source = "#!/bin/sh\ngrep start* out.txt\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedGrepRegex));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("quote the reported grep pattern word")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_grep_pattern_words() {
        let source = "\
#!/bin/sh
grep start* out.txt
grep -e item? out.txt
grep -eitem* out.txt
grep --regexp=foo*bar out.txt
grep -F -- item,[0-4] out.txt
checksum=\"$(grep -Ehrow [0-9a-f]{40} ${template}|sort|uniq|tr '\\n' ' ')\"
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::UnquotedGrepRegex),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 6);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
grep \"start*\" out.txt
grep -e \"item?\" out.txt
grep \"-eitem*\" out.txt
grep \"--regexp=foo*bar\" out.txt
grep -F -- \"item,[0-4]\" out.txt
checksum=\"$(grep -Ehrow \"[0-9a-f]{40}\" ${template}|sort|uniq|tr '\\n' ' ')\"
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_mixed_quoted_grep_pattern_words_preserving_expansions() {
        let source = "\
#!/bin/sh
grep \"$prefix\"[0-9].log out.txt
grep --regexp \"$prefix\"[0-9].log out.txt
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::UnquotedGrepRegex),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
grep \"${prefix}[0-9].log\" out.txt
grep --regexp \"${prefix}[0-9].log\" out.txt
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_non_violating_grep_pattern_words_unchanged_when_fixing() {
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
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::UnquotedGrepRegex),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
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

    #[test]
    fn respects_zsh_glob_activation_for_grep_patterns() {
        let source = "\
#!/usr/bin/env zsh
setopt no_glob
grep start* out.txt
setopt glob extended_glob
grep foo^bar out.txt
grep foo~bar out.txt
grep foo~bar* out.txt
unsetopt extended_glob
grep foo^bar out.txt
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedGrepRegex).with_shell(crate::ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["foo^bar", "foo~bar*"]
        );
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C084.sh").as_path(),
            &LinterSettings::for_rule(Rule::UnquotedGrepRegex),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C084_fix_C084.sh", result);
        Ok(())
    }
}
