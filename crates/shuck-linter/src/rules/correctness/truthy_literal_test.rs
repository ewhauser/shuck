use shuck_ast::{ConditionalUnaryOp, Span};

use crate::{Checker, ConditionalNodeFact, ConditionalOperatorFamily, Rule, Violation};

pub struct TruthyLiteralTest;

impl Violation for TruthyLiteralTest {
    fn rule() -> Rule {
        Rule::TruthyLiteralTest
    }

    fn message(&self) -> String {
        "this test checks a fixed literal instead of runtime data".to_owned()
    }
}

pub fn truthy_literal_test(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            let mut spans = Vec::new();
            if let Some(simple_test) = fact.simple_test() {
                spans.extend(simple_test_report_spans(simple_test, source));
            }
            if let Some(conditional) = fact.conditional() {
                spans.extend(conditional_report_spans(conditional));
            }
            spans
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || TruthyLiteralTest);
}

fn simple_test_report_spans(fact: &crate::SimpleTestFact<'_>, source: &str) -> Vec<Span> {
    fact.truthy_expression_words(source)
        .into_iter()
        .filter_map(|word| {
            fact.effective_operands()
                .iter()
                .position(|operand| operand.span == word.span)
                .and_then(|index| fact.effective_operand_class(index))
                .is_some_and(|class| class.is_fixed_literal())
                .then_some(word.span)
        })
        .collect()
}

fn conditional_report_spans(fact: &crate::ConditionalFact<'_>) -> Vec<Span> {
    let excluded_operand_spans = fact
        .nodes()
        .iter()
        .flat_map(|node| match node {
            ConditionalNodeFact::Unary(unary) if unary.op() != ConditionalUnaryOp::Not => unary
                .operand()
                .word()
                .map(|word| vec![word.span])
                .unwrap_or_default(),
            ConditionalNodeFact::Binary(binary)
                if binary.operator_family() != ConditionalOperatorFamily::Logical =>
            {
                [binary.left().word(), binary.right().word()]
                    .into_iter()
                    .flatten()
                    .map(|word| word.span)
                    .collect::<Vec<_>>()
            }
            ConditionalNodeFact::BareWord(_)
            | ConditionalNodeFact::Unary(_)
            | ConditionalNodeFact::Binary(_)
            | ConditionalNodeFact::Other(_) => Vec::new(),
        })
        .collect::<Vec<_>>();

    fact.nodes()
        .iter()
        .filter_map(|node| match node {
            ConditionalNodeFact::BareWord(word)
                if word.operand().class().is_fixed_literal()
                    && word
                        .operand()
                        .word()
                        .is_some_and(|operand| !excluded_operand_spans.contains(&operand.span)) =>
            {
                word.operand().word().map(|operand| operand.span)
            }
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn ignores_runtime_sensitive_literal_words() {
        let source = "\
#!/bin/bash
[ ~ ]
test ~user
test x=~
test *.sh
[ {a,b} ]
[[ ~ ]]
[[ *.sh ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TruthyLiteralTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![8]
        );
    }

    #[test]
    fn still_reports_plain_fixed_literals() {
        let source = "\
#!/bin/bash
[ 1 ]
test foo
[[ bar ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TruthyLiteralTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![2, 3, 4]
        );
    }

    #[test]
    fn anchors_truthy_simple_tests_on_the_operand() {
        let source = "\
#!/bin/bash
[ \"\" ]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TruthyLiteralTest));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "\"\"");
    }

    #[test]
    fn reports_truthy_terms_inside_simple_test_logical_chains() {
        let source = "\
#!/bin/sh
[ \"$mode\" = yes -o foo ]
[ bar -a \"$mode\" = yes ]
[ \"$mode\" = yes -o \"$other\" = no ]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TruthyLiteralTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["foo", "bar"]
        );
    }

    #[test]
    fn reports_truthy_literals_inside_negated_and_logical_conditionals() {
        let source = "\
#!/bin/bash
[[ ! foo ]]
[[ \"$mode\" == yes || bar ]]
[[ ! -n baz ]]
[[ foo == bar ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TruthyLiteralTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["foo", "bar"]
        );
    }

    #[test]
    fn ignores_tab_stripped_heredoc_substitutions_after_earlier_heredocs() {
        let source = "\
#!/bin/bash
case \"${tag_type}\" in
  newest-tag)
\t:
\t;;
  latest-release-tag)
\t:
\t;;
  latest-regex)
\t:
\t;;
  *)
\ttermux_error_exit <<-EndOfError
\t\tERROR: Invalid TERMUX_PKG_UPDATE_TAG_TYPE: '${tag_type}'.
\t\tAllowed values: 'newest-tag', 'latest-release-tag', 'latest-regex'.
\tEndOfError
\t;;
esac

case \"${http_code}\" in
  404)
\ttermux_error_exit <<-EndOfError
\t\tNo '${tag_type}' found. (${api_url})
\t\tHTTP code: ${http_code}
\t\tTry using '$(
\t\t\tif [[ \"${tag_type}\" == \"newest-tag\" ]]; then
\t\t\t\techo \"latest-release-tag\"
\t\t\telse
\t\t\t\techo \"newest-tag\"
\t\t\tfi
\t\t)'.
\tEndOfError
\t;;
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TruthyLiteralTest));

        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {:?}",
            diagnostics
                .iter()
                .map(|diagnostic| (
                    diagnostic.span.start.line,
                    diagnostic.span.slice(source).to_owned(),
                ))
                .collect::<Vec<_>>()
        );
    }
}
