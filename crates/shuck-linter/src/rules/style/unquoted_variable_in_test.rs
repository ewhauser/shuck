use shuck_ast::static_word_text;

use crate::{
    Checker, ExpansionContext, Rule, SimpleTestShape, SimpleTestSyntax, Violation, WordFactContext,
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
    let source = checker.source();
    checker.report_fact_spans_dedup(
        |facts, report| {
            for fact in facts.commands() {
                let Some(simple_test) = fact.simple_test() else {
                    continue;
                };
                if simple_test.syntax() != SimpleTestSyntax::Bracket {
                    continue;
                }
                if simple_test.shape() != SimpleTestShape::Unary
                    || simple_test.operands().len() != 2
                {
                    continue;
                }

                let operator = simple_test.operands()[0];
                if static_word_text(operator, source).as_deref() != Some("-n") {
                    continue;
                }

                let operand = simple_test.operands()[1];
                let Some(word_fact) = facts.word_fact(
                    operand.span,
                    WordFactContext::Expansion(ExpansionContext::CommandArgument),
                ) else {
                    continue;
                };

                for span in word_fact.unquoted_scalar_expansion_spans().iter().copied() {
                    report(span);
                }
            }
        },
        || UnquotedVariableInTest,
    );
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
[ -n prefix$baz ]
[ -n ${bar} ]
[ -n ${qux:-fallback} ]
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
            vec!["$foo", "$baz", "${bar}", "${qux:-fallback}"]
        );
    }

    #[test]
    fn ignores_quoted_and_non_n_unary_tests() {
        let source = "\
#!/bin/sh
[ -n ${arr[*]} ]
[ -n \"$foo\" ]
test -n $foo
test -z $foo
[ -n literal ]
test -n $(printf '%s\\n' \"$foo\")
test -n ${qux:-fallback}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedVariableInTest),
        );

        assert!(diagnostics.is_empty());
    }
}
