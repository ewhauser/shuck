use crate::facts::MixedShortCircuitKind;
use crate::{Checker, Rule, Violation};

pub struct DefaultElseInShortCircuit;

impl Violation for DefaultElseInShortCircuit {
    fn rule() -> Rule {
        Rule::DefaultElseInShortCircuit
    }

    fn message(&self) -> String {
        "this `||` fallback also runs when the `&&` assignment branch fails".to_owned()
    }
}

pub fn default_else_in_short_circuit(checker: &mut Checker) {
    let spans = checker
        .facts()
        .lists()
        .iter()
        .filter(|list| {
            list.mixed_short_circuit_kind() == Some(MixedShortCircuitKind::AssignmentTernary)
        })
        .filter_map(|list| list.operators().last().map(|operator| operator.span()))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || DefaultElseInShortCircuit);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_the_fallback_operator_and_skips_other_chain_kinds() {
        let source = "\
[ -n \"$str\" ] && out=foo || out=bar
[ -n \"$str\" ] || out=foo && out=bar
[ \"$x\" = foo ] && [ \"$x\" = bar ] || [ \"$x\" = baz ]
cmd && first || second
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::DefaultElseInShortCircuit),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "||");
    }
}
