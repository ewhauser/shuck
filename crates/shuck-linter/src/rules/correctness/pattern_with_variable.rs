use shuck_ast::Span;

use crate::{Checker, ExpansionContext, Rule, Violation};

pub struct PatternWithVariable;

impl Violation for PatternWithVariable {
    fn rule() -> Rule {
        Rule::PatternWithVariable
    }

    fn message(&self) -> String {
        "pattern expressions should not expand variables".to_owned()
    }
}

pub fn pattern_with_variable(checker: &mut Checker) {
    let replacement_expansion_spans = checker
        .facts()
        .replacement_expansion_fragments()
        .iter()
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();
    let special_target_spans = checker
        .facts()
        .parameter_pattern_special_target_fragments()
        .iter()
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();

    let spans = checker
        .facts()
        .expansion_word_facts(ExpansionContext::ParameterPattern)
        .flat_map(|fact| {
            if span_is_within_any(fact.span(), &replacement_expansion_spans)
                || span_is_within_any(fact.span(), &special_target_spans)
            {
                return Vec::new();
            }

            let quoted_expansion_spans = fact.double_quoted_expansion_spans();
            fact.active_expansion_spans()
                .iter()
                .copied()
                .filter(|span| !span_is_within_any(*span, quoted_expansion_spans))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || PatternWithVariable);
}

fn span_is_within_any(span: Span, hosts: &[Span]) -> bool {
    hosts
        .iter()
        .any(|host| host.start.offset <= span.start.offset && span.end.offset <= host.end.offset)
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_nested_parameter_pattern_groups_and_substitutions() {
        let source = "\
#!/bin/bash
suffix=bc
trimmed=${name%@($suffix|$(printf '%s' zz))}
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::PatternWithVariable));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$suffix", "$(printf '%s' zz)"]
        );
    }

    #[test]
    fn ignores_quoted_and_replacement_parameter_patterns() {
        let source = "\
#!/bin/bash
trimmed_one=${name%$trimmed_one_suffix}
trimmed_two=${name##foo$trimmed_two_suffix}
quoted=${name#\"$quoted_suffix\"}
replaced_one=${name/$replaced_one_suffix/x}
replaced_two=${name/foo$replaced_two_suffix/x}
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::PatternWithVariable));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$trimmed_one_suffix", "$trimmed_two_suffix"]
        );
    }

    #[test]
    fn ignores_zero_and_array_trim_targets() {
        let source = "\
#!/bin/bash
trimmed=${name%$trimmed_suffix}
script_name=${0##*/}
all_trimmed=(\"${items[@]#$array_prefix/}\")
one_trimmed=${items[i]%$item_suffix}
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::PatternWithVariable));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$trimmed_suffix"]
        );
    }
}
