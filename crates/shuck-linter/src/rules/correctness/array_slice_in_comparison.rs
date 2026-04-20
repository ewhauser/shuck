use shuck_ast::Span;

use crate::{
    Checker, ConditionalNodeFact, ConditionalOperatorFamily, ExpansionContext, Rule, Violation,
};

pub struct ArraySliceInComparison;

impl Violation for ArraySliceInComparison {
    fn rule() -> Rule {
        Rule::ArraySliceInComparison
    }

    fn message(&self) -> String {
        "all-elements array expansions collapse inside `[[ ... ]]` tests".to_owned()
    }
}

pub fn array_slice_in_comparison(checker: &mut Checker) {
    let direct_operand_spans = [
        ExpansionContext::StringTestOperand,
        ExpansionContext::RegexOperand,
    ]
        .into_iter()
        .flat_map(|context| checker.facts().expansion_word_facts(context))
        .filter(|fact| !fact.is_nested_word_command())
        .filter(|fact| fact.has_direct_all_elements_array_expansion_in_source(checker.source()))
        .map(|fact| fact.span())
        .collect::<Vec<_>>();

    let risky_pattern_word_spans = checker
        .facts()
        .expansion_word_facts(ExpansionContext::ConditionalPattern)
        .filter(|fact| !fact.is_nested_word_command())
        .filter(|fact| fact.command_substitution_spans().is_empty())
        .filter(|fact| !fact.is_pure_positional_at_splat())
        .filter(|fact| fact.has_direct_all_elements_array_expansion_in_source(checker.source()))
        .map(|fact| fact.span())
        .collect::<Vec<_>>();

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| fact.conditional())
        .flat_map(|conditional| conditional.nodes().iter())
        .flat_map(|node| conditional_pattern_spans(node, &risky_pattern_word_spans))
        .chain(direct_operand_spans)
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ArraySliceInComparison);
}

fn conditional_pattern_spans(
    fact: &ConditionalNodeFact<'_>,
    risky_word_spans: &[Span],
) -> Vec<Span> {
    match fact {
        ConditionalNodeFact::Binary(binary)
            if binary.operator_family() != ConditionalOperatorFamily::Logical =>
        {
            [
                pattern_span_if_risky(binary.left().expression().span(), risky_word_spans),
                pattern_span_if_risky(binary.right().expression().span(), risky_word_spans),
            ]
            .into_iter()
            .flatten()
            .collect()
        }
        ConditionalNodeFact::BareWord(_)
        | ConditionalNodeFact::Unary(_)
        | ConditionalNodeFact::Binary(_)
        | ConditionalNodeFact::Other(_) => Vec::new(),
    }
}

fn pattern_span_if_risky(span: Span, risky_word_spans: &[Span]) -> Option<Span> {
    risky_word_spans
        .iter()
        .copied()
        .any(|word_span| span_contains(span, word_span))
        .then_some(span)
}

fn span_contains(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && outer.end.offset >= inner.end.offset
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_all_elements_array_expansions_in_double_bracket_tests() {
        let source = "\
#!/bin/bash
set -- a b
arr=(x y)
if [[ \"${sel[@]:0:4}\" == \"HELP\" ]]; then :; fi
if [[ -n \"$@\" ]]; then :; fi
if [[ x == *${arr[@]}* ]]; then :; fi
if [[ \"${@: -1}\" == \"mM\" || \"${@:-1}\" == \"Mm\" ]]; then :; fi
if [[ \" ${arr[@]} \" =~ \" x \" ]]; then :; fi
if [[ \"${arr[@]}\" ]]; then :; fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArraySliceInComparison),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "\"${sel[@]:0:4}\"",
                "\"$@\"",
                "*${arr[@]}*",
                "\"${@: -1}\"",
                "\"${@:-1}\"",
                "\" ${arr[@]} \"",
                "\"${arr[@]}\"",
            ]
        );
    }

    #[test]
    fn ignores_star_expansions_escaped_literals_and_single_bracket_tests() {
        let source = "\
#!/bin/bash
if [[ \"${sel[*]:1}\" == \"HELP\" ]]; then :; fi
if [[ \"\\${sel[@]:1}\" == \"HELP\" ]]; then :; fi
if [[ x == ${sel[*]}* ]]; then :; fi
if [[ \"\\$@\" ]]; then :; fi
if [[ -z ${packed=\"$@\"} ]]; then :; fi
if [ \"${sel[@]:1}\" = \"HELP\" ]; then :; fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArraySliceInComparison),
        );

        assert!(diagnostics.is_empty());
    }
}
