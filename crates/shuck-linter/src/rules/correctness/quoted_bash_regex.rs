use crate::rules::common::word::{
    TestOperandClass, WordQuote, classify_contextual_operand, classify_word, static_word_text,
};
use crate::rules::common::{
    expansion::ExpansionContext,
    query::{self, CommandWalkOptions},
};
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
    let mut spans = Vec::new();

    query::walk_commands(
        &checker.ast().body,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |visit| {
            let _command = visit.command;
            query::visit_expansion_words(visit, source, &mut |word, context| {
                if context != ExpansionContext::RegexOperand {
                    return;
                }

                if classify_word(word, source).quote != WordQuote::Unquoted
                    && quoted_regex_requires_warning(word, source)
                {
                    spans.push(word.span);
                }
            });
        },
    );

    for span in spans {
        checker.report(QuotedBashRegex, span);
    }
}

fn quoted_regex_requires_warning(word: &shuck_ast::Word, source: &str) -> bool {
    match classify_contextual_operand(word, source, ExpansionContext::RegexOperand) {
        TestOperandClass::RuntimeSensitive => true,
        TestOperandClass::FixedLiteral => static_word_text(word, source)
            .is_some_and(|text| literal_uses_regex_significance(&text)),
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
}
