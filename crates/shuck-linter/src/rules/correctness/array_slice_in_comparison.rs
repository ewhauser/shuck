use crate::{
    Checker, ConditionalNodeFact, ConditionalOperatorFamily, Rule, Violation,
    word_all_elements_array_slice_span_in_source,
};

pub struct ArraySliceInComparison;

impl Violation for ArraySliceInComparison {
    fn rule() -> Rule {
        Rule::ArraySliceInComparison
    }

    fn message(&self) -> String {
        "array-slice expansions collapse when used in string comparisons".to_owned()
    }
}

pub fn array_slice_in_comparison(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| {
            fact.conditional()
                .and_then(|conditional| conditional_span(conditional.root(), checker.source()))
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ArraySliceInComparison);
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
        .and_then(|word| word_all_elements_array_slice_span_in_source(word, source))
        .or_else(|| {
            binary
                .right()
                .word()
                .and_then(|word| word_all_elements_array_slice_span_in_source(word, source))
        })
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_array_slices_in_double_bracket_string_comparisons() {
        let source = "\
#!/bin/bash
if [[ \"${sel[@]:0:4}\" == \"HELP\" ]]; then :; fi
if [[ \"x${@:2}y\" == \"x\" ]]; then :; fi
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
            vec!["${sel[@]:0:4}", "${@:2}"]
        );
    }

    #[test]
    fn ignores_non_slice_or_non_conditional_comparisons() {
        let source = "\
#!/bin/bash
if [[ \"${sel[@]}\" == \"HELP\" ]]; then :; fi
if [[ \"${sel[*]:1}\" == \"HELP\" ]]; then :; fi
if [[ \"\\${sel[@]:1}\" == \"HELP\" ]]; then :; fi
if [ \"${sel[@]:1}\" = \"HELP\" ]; then :; fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArraySliceInComparison),
        );

        assert!(diagnostics.is_empty());
    }
}
