use crate::facts::MixedShortCircuitKind;
use crate::{Checker, Rule, Violation};

pub struct ChainedTestBranches;

impl Violation for ChainedTestBranches {
    fn rule() -> Rule {
        Rule::ChainedTestBranches
    }

    fn message(&self) -> String {
        "chaining `&&` and `||` makes the fallback depend on the middle command status".to_owned()
    }
}

pub fn chained_test_branches(checker: &mut Checker) {
    let spans = checker
        .facts()
        .lists()
        .iter()
        .filter(|list| list.mixed_short_circuit_kind() == Some(MixedShortCircuitKind::TestChain))
        .filter_map(|list| list.mixed_short_circuit_span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ChainedTestBranches);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_the_operator_that_introduces_mixed_short_circuiting() {
        let source = "\
[ \"$x\" = foo ] && [ \"$x\" = bar ] || [ \"$x\" = baz ]
false || true && [ \"$x\" = baz ]
true && false; false || printf '%s\\n' ok
[ -n \"$x\" ] && out=foo || out=bar
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::ChainedTestBranches));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["&&", "||"]
        );
    }
}
