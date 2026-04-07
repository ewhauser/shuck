use crate::rules::common::word::{TestOperandClass, WordQuote, static_word_text};
use crate::{Checker, Rule, Violation};

pub struct QuotedBashRegex;

impl Violation for QuotedBashRegex {
    fn rule() -> Rule {
        Rule::QuotedBashRegex
    }

    fn message(&self) -> String {
        "quoting the right-hand side of `=~` forces a literal string match".to_owned()
    }
}

pub fn quoted_bash_regex(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| fact.conditional())
        .flat_map(|fact| fact.regex_nodes())
        .filter_map(|regex| {
            let right = regex.right();
            let word = right.word()?;
            if right.quote() == Some(WordQuote::Unquoted) {
                return None;
            }

            let should_report = match right.class() {
                TestOperandClass::RuntimeSensitive => true,
                TestOperandClass::FixedLiteral => static_word_text(word, source)
                    .is_some_and(|text| literal_uses_regex_significance(&text)),
            };

            should_report.then_some(word.span)
        })
        .collect::<Vec<_>>();

    for span in spans {
        checker.report_dedup(QuotedBashRegex, span);
    }
}

fn literal_uses_regex_significance(text: &str) -> bool {
    let mut escaped = false;

    for char in text.chars() {
        if escaped {
            return true;
        }

        if char == '\\' {
            escaped = true;
            continue;
        }

        if matches!(
            char,
            '.' | '[' | ']' | '(' | ')' | '{' | '}' | '*' | '+' | '?' | '|' | '^' | '$'
        ) {
            return true;
        }
    }

    escaped
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn ignores_quoted_fixed_literals_without_regex_semantics() {
        let source = "\
#!/bin/bash
[[ \"$output\" =~ \"Error: No available formula\" ]]
[[ \"$output\" =~ \"~user\" ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashRegex));

        assert!(diagnostics.is_empty());
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
    fn keeps_reporting_mixed_quoted_regex_operands() {
        let source = "#!/bin/bash\n[[ $value =~ ^\"foo\"bar$ ]]\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashRegex));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "^\"foo\"bar$");
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
}
