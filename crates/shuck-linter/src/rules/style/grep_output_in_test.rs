use shuck_ast::Span;

use crate::{
    Checker, CommandSubstitutionKind, ConditionalNodeFact, ConditionalOperatorFamily,
    ExpansionContext, Rule, SimpleTestSyntax, SubstitutionFact, Violation, WordFactContext,
};

pub struct GrepOutputInTest;

impl Violation for GrepOutputInTest {
    fn rule() -> Rule {
        Rule::GrepOutputInTest
    }

    fn message(&self) -> String {
        "use grep's exit status instead of testing its output".to_owned()
    }
}

pub fn grep_output_in_test(checker: &mut Checker) {
    let substitutions = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| fact.substitution_facts().iter())
        .collect::<Vec<_>>();

    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            let mut spans = Vec::new();
            if let Some(simple_test) = fact.simple_test() {
                spans.extend(collect_simple_test_spans(
                    checker,
                    simple_test,
                    &substitutions,
                ));
            }
            if let Some(conditional) = fact.conditional() {
                spans.extend(collect_conditional_spans(
                    conditional,
                    checker.source(),
                    &substitutions,
                ));
            }
            spans
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || GrepOutputInTest);
}

fn collect_simple_test_spans(
    checker: &Checker<'_>,
    simple_test: &crate::SimpleTestFact<'_>,
    substitutions: &[&SubstitutionFact],
) -> Vec<Span> {
    if simple_test.syntax() != SimpleTestSyntax::Bracket {
        return Vec::new();
    }

    let source = checker.source();
    let mut spans = simple_test
        .truthy_expression_words(source)
        .into_iter()
        .filter(|word| word_fact_has_plain_command_substitution(checker, word.span, substitutions))
        .map(|word| shellcheck_truthy_test_span(source, word.span))
        .collect::<Vec<_>>();

    spans.extend(
        simple_test
            .string_unary_expression_words(source)
            .into_iter()
            .filter_map(|(operator, operand)| {
                word_fact_has_plain_command_substitution(checker, operand.span, substitutions)
                    .then_some(operator.span)
            }),
    );

    spans
}

fn collect_conditional_spans(
    conditional: &crate::ConditionalFact<'_>,
    source: &str,
    substitutions: &[&SubstitutionFact],
) -> Vec<Span> {
    let mut spans = Vec::new();
    let unary_operand_spans = conditional
        .nodes()
        .iter()
        .filter_map(|node| match node {
            ConditionalNodeFact::Unary(unary)
                if unary.operator_family() == ConditionalOperatorFamily::StringUnary =>
            {
                unary.operand().word().map(|word| word.span)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let comparison_operand_spans = conditional
        .nodes()
        .iter()
        .filter_map(|node| match node {
            ConditionalNodeFact::Binary(binary)
                if binary.operator_family() != ConditionalOperatorFamily::Logical =>
            {
                Some([
                    binary.left().word().map(|word| word.span),
                    binary.right().word().map(|word| word.span),
                ])
            }
            _ => None,
        })
        .flatten()
        .flatten()
        .collect::<Vec<_>>();

    match conditional.root() {
        ConditionalNodeFact::BareWord(bare_word)
            if bare_word.operand().word().is_some_and(|word| {
                bare_word
                    .operand()
                    .word_classification()
                    .is_some_and(|classification| {
                        classification.has_plain_command_substitution()
                            && word_has_top_level_grep_substitution(word.span, substitutions)
                    })
            }) =>
        {
            if let Some(word) = bare_word.operand().word() {
                spans.push(shellcheck_truthy_test_span(source, word.span));
            }
        }
        ConditionalNodeFact::Unary(unary)
            if unary.operator_family() == ConditionalOperatorFamily::StringUnary
                && unary.operand().word().is_some_and(|word| {
                    unary
                        .operand()
                        .word_classification()
                        .is_some_and(|classification| {
                            classification.has_plain_command_substitution()
                                && word_has_top_level_grep_substitution(word.span, substitutions)
                        })
                }) =>
        {
            spans.push(unary.operator_span());
        }
        _ => {}
    }

    for node in conditional.nodes().iter().skip(1) {
        match node {
            ConditionalNodeFact::BareWord(bare_word)
                if bare_word.operand().word().is_some_and(|word| {
                    !unary_operand_spans.contains(&word.span)
                        && !comparison_operand_spans.contains(&word.span)
                        && bare_word
                            .operand()
                            .word_classification()
                            .is_some_and(|classification| {
                                classification.has_plain_command_substitution()
                                    && word_has_top_level_grep_substitution(
                                        word.span,
                                        substitutions,
                                    )
                            })
                }) =>
            {
                if let Some(word) = bare_word.operand().word() {
                    spans.push(shellcheck_truthy_test_span(source, word.span));
                }
            }
            ConditionalNodeFact::Unary(unary)
                if unary.operator_family() == ConditionalOperatorFamily::StringUnary
                    && unary.operand().word().is_some_and(|word| {
                        unary
                            .operand()
                            .word_classification()
                            .is_some_and(|classification| {
                                classification.has_plain_command_substitution()
                                    && word_has_top_level_grep_substitution(
                                        word.span,
                                        substitutions,
                                    )
                            })
                    }) =>
            {
                spans.push(unary.operator_span());
            }
            _ => {}
        }
    }

    spans
}

fn shellcheck_truthy_test_span(source: &str, span: Span) -> Span {
    let Some(next) = source
        .get(span.end.offset..)
        .and_then(|tail| tail.chars().next())
    else {
        return span;
    };
    if !matches!(next, ' ' | '\t') {
        return span;
    }

    let end_offset = span.end.offset + next.len_utf8();
    let separator = &source[span.end.offset..end_offset];
    Span::from_positions(span.start, span.end.advanced_by(separator))
}

fn word_fact_has_plain_command_substitution(
    checker: &Checker<'_>,
    word_span: Span,
    substitutions: &[&SubstitutionFact],
) -> bool {
    checker
        .facts()
        .word_fact(
            word_span,
            WordFactContext::Expansion(ExpansionContext::CommandArgument),
        )
        .is_some_and(|fact| {
            fact.classification().has_plain_command_substitution()
                && word_has_top_level_grep_substitution(word_span, substitutions)
        })
}

fn word_has_top_level_grep_substitution(
    word_span: Span,
    substitutions: &[&SubstitutionFact],
) -> bool {
    let candidates = substitutions
        .iter()
        .copied()
        .filter(|fact| {
            fact.kind() == CommandSubstitutionKind::Command && fact.host_word_span() == word_span
        })
        .collect::<Vec<_>>();

    candidates.iter().any(|substitution| {
        !substitution.uses_backtick_syntax()
            && substitution.body_contains_grep()
            && !candidates.iter().any(|other| {
                other.span() != substitution.span()
                    && span_contains(other.span(), substitution.span())
            })
    })
}

fn span_contains(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && outer.end.offset >= inner.end.offset
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_grep_output_in_logical_string_tests() {
        let source = "\
#!/bin/bash
[[ -n \"$1\" && ! -f \"$1\" && -n \"$(echo \"$1\" | grep -v '^-')\" ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GrepOutputInTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-n"]
        );
    }

    #[test]
    fn ignores_legacy_backticks_in_simple_tests() {
        let source = "\
#!/bin/sh
[ -z `nvm ls | grep '^ *\\.'` ]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GrepOutputInTest));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_grep_output_in_non_root_bareword_conditionals() {
        let source = "\
#!/bin/bash
[[ \"$ok\" && $(grep foo file) ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GrepOutputInTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$(grep foo file) "]
        );
    }

    #[test]
    fn ignores_legacy_backticks_in_conditionals() {
        let source = "\
#!/bin/bash
[[ -n `go env 2>/dev/null | grep proxy.golang.org` ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GrepOutputInTest));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_grep_output_when_compared_in_binary_conditionals() {
        let source = "\
#!/bin/bash
[[ $(grep foo input.txt) = bar ]]
[[ $(grep foo input.txt) != \"\" ]]
[[ $(grep foo input.txt) -ge 1 ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GrepOutputInTest));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_negated_truthy_simple_tests() {
        let source = "\
#!/bin/sh
[ ! \"$(grep foo input.txt)\" ]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GrepOutputInTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["\"$(grep foo input.txt)\" "]
        );
    }

    #[test]
    fn reports_string_unary_subexpressions_in_compound_simple_tests() {
        let source = "\
#!/bin/sh
[ -f \"$path\" -a ! -z \"$(grep foo input.txt)\" ]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GrepOutputInTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-z"]
        );
    }

    #[test]
    fn reports_quoted_string_unary_operators_in_simple_tests() {
        let source = "\
#!/bin/sh
[ \"-n\" \"$(grep foo input.txt)\" ]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GrepOutputInTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["\"-n\""]
        );
    }

    #[test]
    fn reports_quoted_logical_connectors_in_simple_tests() {
        let source = "\
#!/bin/sh
[ foo \"-o\" \"$(grep foo input.txt)\" ]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GrepOutputInTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["\"$(grep foo input.txt)\" "]
        );
    }

    #[test]
    fn ignores_unary_a_and_o_tests() {
        let source = "\
#!/bin/sh
[ -a \"$(grep foo input.txt)\" ]
[ -o \"$(grep foo input.txt)\" ]
[ ! -a \"$(grep foo input.txt)\" ]
[ ! -o \"$(grep foo input.txt)\" ]
[ -a \"$path\" -o -z \"$ok\" ]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GrepOutputInTest));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_test_builtin_string_unary_forms() {
        let source = "\
#!/bin/sh
test -n \"$(grep foo input.txt)\"
test -z \"$(grep foo input.txt)\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GrepOutputInTest));

        assert!(diagnostics.is_empty());
    }
}
