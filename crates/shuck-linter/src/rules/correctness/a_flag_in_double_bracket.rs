use shuck_ast::ConditionalUnaryOp;

use crate::{Checker, ConditionalNodeFact, Rule, Violation};

pub struct AFlagInDoubleBracket;

impl Violation for AFlagInDoubleBracket {
    fn rule() -> Rule {
        Rule::AFlagInDoubleBracket
    }

    fn message(&self) -> String {
        "use `-e` or `&&` instead of `-a` inside `[[ ... ]]`".to_owned()
    }
}

pub fn a_flag_in_double_bracket(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| fact.conditional())
        .flat_map(|conditional| {
            conditional
                .nodes()
                .iter()
                .filter_map(|node| match node {
                    ConditionalNodeFact::Unary(unary)
                        if unary.op() == ConditionalUnaryOp::Exists
                            && unary.operator_span().slice(source) == "-a" =>
                    {
                        Some(unary.operator_span())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || AFlagInDoubleBracket);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_double_bracket_a_flags() {
        let source = "\
#!/bin/sh
[[ -a \"$path\" ]]
[[ ! -a \"$path\" ]]
[[ -a \"$path\" && -e \"$other\" ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AFlagInDoubleBracket),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-a", "-a", "-a"]
        );
    }

    #[test]
    fn ignores_bracket_test_a_flags_and_other_unary_ops() {
        let source = "\
#!/bin/sh
[ -a \"$path\" ]
[[ -e \"$path\" ]]
[[ -o noclobber ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AFlagInDoubleBracket),
        );

        assert!(diagnostics.is_empty());
    }
}
