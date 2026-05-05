use shuck_ast::Span;

use crate::{Checker, Edit, ExpansionContext, Fix, FixAvailability, Rule, Violation};

pub struct PatternWithVariable;

impl Violation for PatternWithVariable {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::PatternWithVariable
    }

    fn message(&self) -> String {
        "pattern expressions should not expand variables".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("quote the expansion in the pattern".to_owned())
    }
}

pub fn pattern_with_variable(checker: &mut Checker) {
    let source = checker.source();
    let replacement_expansion_spans = checker
        .facts()
        .replacement_expansion_fragments()
        .iter()
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();
    let special_target_operand_spans = checker
        .facts()
        .parameter_pattern_special_target_fragments()
        .iter()
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();

    let diagnostics = checker
        .facts()
        .expansion_word_facts(ExpansionContext::ParameterPattern)
        .flat_map(|fact| {
            if span_is_within_any(fact.span(), &replacement_expansion_spans)
                || special_target_operand_spans.contains(&fact.span())
            {
                return Vec::new();
            }

            let quoted_expansion_spans = fact.double_quoted_expansion_spans();
            fact.active_expansion_spans()
                .iter()
                .copied()
                .filter(|span| !span_is_within_any(*span, quoted_expansion_spans))
                .filter(|span| !fact.expansion_span_is_zsh_force_glob_parameter(*span))
                .map(|span| {
                    crate::Diagnostic::new(PatternWithVariable, span).with_fix(Fix::unsafe_edit(
                        Edit::replacement(format!("\"{}\"", span.slice(source)), span),
                    ))
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn span_is_within_any(span: Span, hosts: &[Span]) -> bool {
    hosts
        .iter()
        .any(|host| host.start.offset <= span.start.offset && span.end.offset <= host.end.offset)
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::test::{test_path_with_fix, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

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

    #[test]
    fn skips_zsh_explicit_pattern_expansion_operands() {
        let source = "\
#!/usr/bin/env zsh
value=foobar
prefix='foo*'
trimmed=${value#${~prefix}}
long_trimmed=${value##${=~prefix}}
suffix_trimmed=${value%${~~~suffix}}
literal_trimmed=${value#${~~literal}}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PatternWithVariable)
                .with_shell(crate::ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${~~literal}"]
        );
    }

    #[test]
    fn reports_nested_patterns_inside_special_targets() {
        let source = "\
#!/bin/bash
nested=${items[i]%${name%$suffix}}
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::PatternWithVariable));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$suffix"]
        );
    }

    #[test]
    fn attaches_unsafe_fix_metadata_to_reported_expansions() {
        let source = "\
#!/bin/bash
suffix=b
trimmed=${name%$suffix}
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::PatternWithVariable));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("quote the expansion in the pattern")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_parameter_pattern_expansions() {
        let source = "\
#!/bin/bash
suffix=b
trimmed=${name%$suffix}
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::PatternWithVariable),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
suffix=b
trimmed=${name%\"$suffix\"}
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            std::path::Path::new("correctness")
                .join("C055.sh")
                .as_path(),
            &LinterSettings::for_rule(Rule::PatternWithVariable),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C055_fix_C055.sh", result);
        Ok(())
    }
}
