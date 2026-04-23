use crate::{
    Checker, Edit, Fix, FixAvailability, Rule, TestOperandClass, Violation, WordQuote,
    static_word_text,
};

pub struct QuotedBashRegex;

impl Violation for QuotedBashRegex {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::QuotedBashRegex
    }

    fn message(&self) -> String {
        "quoting the right-hand side of `=~` forces a literal string match".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove the surrounding quotes from the regex operand".to_owned())
    }
}

pub fn quoted_bash_regex(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| fact.conditional())
        .flat_map(|fact| fact.regex_nodes())
        .filter_map(|regex| {
            let right = regex.right();
            let word = right.word()?;
            let static_text = static_word_text(word, source);
            if right.quote() != Some(WordQuote::FullyQuoted) {
                return None;
            }

            let should_report = match right.class() {
                TestOperandClass::RuntimeSensitive => static_text
                    .as_deref()
                    .is_none_or(literal_uses_regex_significance),
                TestOperandClass::FixedLiteral => static_text
                    .as_deref()
                    .is_some_and(literal_uses_regex_significance),
            };

            should_report.then(|| {
                let diagnostic = crate::Diagnostic::new(QuotedBashRegex, word.span);
                match quoted_bash_regex_fix(word, source) {
                    Some(fix) => diagnostic.with_fix(fix),
                    None => diagnostic,
                }
            })
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn quoted_bash_regex_fix(word: &shuck_ast::Word, source: &str) -> Option<Fix> {
    let content_span = word.quoted_content_span_in_source(source)?;
    Some(Fix::unsafe_edits([
        Edit::deletion_at(word.span.start.offset, content_span.start.offset),
        Edit::deletion_at(content_span.end.offset, word.span.end.offset),
    ]))
}

fn literal_uses_regex_significance(text: &str) -> bool {
    let mut chars = text.chars().peekable();

    while let Some(char) = chars.next() {
        if matches!(char, '.' | '[' | ']' | '(' | ')' | '*' | '+' | '|' | '$') {
            return true;
        }

        if char == '\\'
            && matches!(
                chars.peek(),
                Some('.' | '[' | ']' | '(' | ')' | '*' | '+' | '|' | '$')
            )
        {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn ignores_quoted_fixed_literals_without_regex_semantics() {
        let source = "\
#!/bin/bash
[[ \"$output\" =~ \"Error: No available formula\" ]]
[[ \"$output\" =~ \"~user\" ]]
[[ \"$output\" =~ \"{\" ]]
[[ \"$output\" =~ $'\\n' ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashRegex));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn attaches_unsafe_fix_metadata_for_plain_quoted_regex_operands() {
        let source = "#!/bin/bash\n[[ foo =~ \"a+\" ]]\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashRegex));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("remove the surrounding quotes from the regex operand")
        );
    }

    #[test]
    fn keeps_reporting_runtime_and_regex_significant_operands() {
        let source = "#!/bin/bash\nre='a+'\n[[ $value =~ \"$re\" ]]\n[[ foo =~ \"a+\" ]]\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashRegex));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![3, 4]
        );
    }

    #[test]
    fn ignores_quoted_non_regex_string_test_operands() {
        let source = "#!/bin/bash\n[[ \"$left\" = \"$right\" ]]\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashRegex));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_mixed_quoted_regex_operands() {
        let source = "#!/bin/bash\n[[ $value =~ ^\"foo\"bar$ ]]\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashRegex));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_plain_quoted_regex_operands() {
        let source = "\
#!/bin/bash
re='a+'
[[ $value =~ \"$re\" ]]
[[ foo =~ \"a+\" ]]
[[ $value =~ 'a+' ]]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::QuotedBashRegex),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
re='a+'
[[ $value =~ $re ]]
[[ foo =~ a+ ]]
[[ $value =~ a+ ]]
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_dollar_quoted_regex_operands_unchanged_when_fixing() {
        let source = "\
#!/bin/bash
[[ foo =~ $\"a+\" ]]
[[ foo =~ $'a+' ]]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::QuotedBashRegex),
            Applicability::Unsafe,
        );

        assert_eq!(result.diagnostics.len(), 2);
        assert!(
            result
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.fix.is_none())
        );
        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert_eq!(
            result
                .fixed_diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$\"a+\"", "$'a+'"]
        );
    }

    #[test]
    fn reports_nested_regex_matches_inside_logical_expressions() {
        let source = "#!/bin/bash\n[[ \"$left\" = right && $value =~ \"$re\" ]]\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashRegex));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "\"$re\"");
    }

    #[test]
    fn reports_nested_regex_matches_inside_command_substitutions() {
        let source = "#!/bin/bash\nprintf '%s\\n' \"$( [[ $value =~ \"$re\" ]] && echo ok )\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashRegex));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "\"$re\"");
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C009.sh").as_path(),
            &LinterSettings::for_rule(Rule::QuotedBashRegex),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C009_fix_C009.sh", result);
        Ok(())
    }
}
