use crate::{
    Checker, ConditionalNodeFact, ConditionalOperatorFamily, Rule, SimpleTestOperatorFamily,
    SimpleTestShape, Violation, word_positional_at_splat_span_in_source,
};

pub struct AtSignInStringCompare;

impl Violation for AtSignInStringCompare {
    fn rule() -> Rule {
        Rule::AtSignInStringCompare
    }

    fn message(&self) -> String {
        "positional-parameter at-splats fold arguments in string comparisons".to_owned()
    }
}

pub fn at_sign_in_string_compare(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| {
            if let Some(simple_test) = fact.simple_test()
                && let Some(span) = simple_test_span(simple_test, checker.source())
            {
                return Some(span);
            }

            fact.conditional()
                .and_then(|conditional| conditional_span(conditional.root(), checker.source()))
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || AtSignInStringCompare);
}

fn simple_test_span(fact: &crate::SimpleTestFact<'_>, source: &str) -> Option<shuck_ast::Span> {
    if fact.shape() != SimpleTestShape::Binary
        || fact.operator_family() != SimpleTestOperatorFamily::StringBinary
    {
        return None;
    }

    fact.operands()
        .first()
        .and_then(|word| word_positional_at_splat_span_in_source(word, source))
        .or_else(|| {
            fact.operands()
                .get(2)
                .and_then(|word| word_positional_at_splat_span_in_source(word, source))
        })
}

fn conditional_span(fact: &ConditionalNodeFact<'_>, source: &str) -> Option<shuck_ast::Span> {
    let ConditionalNodeFact::Binary(binary) = fact else {
        return None;
    };
    if binary.operator_family() != ConditionalOperatorFamily::StringBinary {
        return None;
    }

    binary
        .left()
        .word()
        .and_then(|word| word_positional_at_splat_span_in_source(word, source))
        .or_else(|| {
            binary
                .right()
                .word()
                .and_then(|word| word_positional_at_splat_span_in_source(word, source))
        })
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_positional_at_splats_in_string_comparisons() {
        let source = "\
#!/bin/bash
if [ \"_$@\" = \"_--version\" ]; then :; fi
if [ \"$@\" = \"--version\" ]; then :; fi
if [ \"${@:-fallback}\" = \"--version\" ]; then :; fi
if [[ \"_$@\" == \"_--version\" ]]; then :; fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AtSignInStringCompare),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$@", "$@", "${@:-fallback}", "$@"]
        );
    }

    #[test]
    fn ignores_non_positional_or_escaped_comparisons() {
        let source = "\
#!/bin/bash
if [ \"_${arr[@]}\" = \"_x\" ]; then :; fi
if [ \"_${arr[@]:1}\" = \"_x\" ]; then :; fi
if [ \"\\$@\" = \"x\" ]; then :; fi
if [[ \"\\$@\" == \"x\" ]]; then :; fi
if [ \"_$*\" = \"_--version\" ]; then :; fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AtSignInStringCompare),
        );

        assert!(diagnostics.is_empty());
    }
}
