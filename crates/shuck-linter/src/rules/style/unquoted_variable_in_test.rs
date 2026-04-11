use crate::{
    Checker, ExpansionContext, Rule, SimpleTestShape, Violation, WordFactContext, static_word_text,
};

pub struct UnquotedVariableInTest;

impl Violation for UnquotedVariableInTest {
    fn rule() -> Rule {
        Rule::UnquotedVariableInTest
    }

    fn message(&self) -> String {
        "quote variable expansions in -n tests".to_owned()
    }
}

pub fn unquoted_variable_in_test(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            let Some(simple_test) = fact.simple_test() else {
                return Vec::new();
            };
            if simple_test.shape() != SimpleTestShape::Unary || simple_test.operands().len() != 2 {
                return Vec::new();
            }

            let operator = simple_test.operands()[0];
            if static_word_text(operator, checker.source()).as_deref() != Some("-n") {
                return Vec::new();
            }

            let operand = simple_test.operands()[1];
            let Some(word_fact) = checker.facts().word_fact(
                operand.span,
                WordFactContext::Expansion(ExpansionContext::CommandArgument),
            ) else {
                return Vec::new();
            };

            word_fact
                .unquoted_scalar_expansion_spans()
                .iter()
                .copied()
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || UnquotedVariableInTest);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_unquoted_scalar_expansions_in_n_tests() {
        let source = "\
#!/bin/sh
[ -n $foo ]
test -n ${bar}
[ -n prefix$baz ]
test -n ${qux:-fallback}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedVariableInTest),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$foo", "${bar}", "$baz", "${qux:-fallback}"]
        );
    }

    #[test]
    fn ignores_quoted_and_non_n_unary_tests() {
        let source = "\
#!/bin/sh
[ -n \"$foo\" ]
test -z $foo
[ -n literal ]
test -n $(printf '%s\\n' \"$foo\")
[ -n ${arr[*]} ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedVariableInTest),
        );

        assert!(diagnostics.is_empty());
    }
}
