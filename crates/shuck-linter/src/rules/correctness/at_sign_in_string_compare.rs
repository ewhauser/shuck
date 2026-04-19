use shuck_ast::Span;

use crate::{Checker, Rule, Violation, word_positional_at_splat_span_in_source};

pub struct AtSignInStringCompare;

impl Violation for AtSignInStringCompare {
    fn rule() -> Rule {
        Rule::AtSignInStringCompare
    }

    fn message(&self) -> String {
        "positional-parameter at-splats fold arguments when used as test operands".to_owned()
    }
}

pub fn at_sign_in_string_compare(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| fact.simple_test())
        .flat_map(|simple_test| simple_test_spans(simple_test, source))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || AtSignInStringCompare);
}

fn simple_test_spans(fact: &crate::SimpleTestFact<'_>, source: &str) -> Vec<Span> {
    fact.operator_expression_operand_words(source)
        .into_iter()
        .filter_map(|word| word_positional_at_splat_span_in_source(word, source).map(|_| word.span))
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_positional_at_splats_in_test_operands() {
        let source = "\
#!/bin/bash
if [ -z \"$@\" ]; then :; fi
if test -n \"${@:-fallback}\"; then :; fi
if [ -d \"$@\" ]; then :; fi
if [ \"_$@\" = \"_--version\" ]; then :; fi
if [ \"$@\" = \"--version\" ]; then :; fi
if [ ! \"$@\" = \"x\" ]; then :; fi
if [ -n foo -a \"${@:-lhs}\" = \"${@:-rhs}\" ]; then :; fi
if [ -d \"$@\" -o \"${@:-fallback}\" = \"x\" ]; then :; fi
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
            vec![
                "\"$@\"",
                "\"${@:-fallback}\"",
                "\"$@\"",
                "\"_$@\"",
                "\"$@\"",
                "\"$@\"",
                "\"${@:-lhs}\"",
                "\"${@:-rhs}\"",
                "\"$@\"",
                "\"${@:-fallback}\"",
            ]
        );
    }

    #[test]
    fn ignores_non_positional_truthy_double_bracket_and_escaped_tests() {
        let source = "\
#!/bin/bash
if [ \"$@\" ]; then :; fi
if test \"${@:-fallback}\"; then :; fi
if [ \"_${arr[@]}\" = \"_x\" ]; then :; fi
if [ \"_${arr[@]:1}\" = \"_x\" ]; then :; fi
if [ \"\\$@\" = \"x\" ]; then :; fi
if [[ \"_$@\" == \"_x\" ]]; then :; fi
if [ ! \"\\$@\" = \"x\" ]; then :; fi
if [ \"_$*\" = \"_--version\" ]; then :; fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AtSignInStringCompare),
        );

        assert!(diagnostics.is_empty());
    }
}
