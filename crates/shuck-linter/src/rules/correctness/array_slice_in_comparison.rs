use shuck_ast::Span;

use crate::{
    Checker, ConditionalNodeFact, ConditionalOperatorFamily, Diagnostic, Edit, ExpansionContext,
    Fix, FixAvailability, Rule, Violation,
};

pub struct ArraySliceInComparison;

impl Violation for ArraySliceInComparison {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::ArraySliceInComparison
    }

    fn message(&self) -> String {
        "all-elements array expansions collapse inside `[[ ... ]]` tests".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("rewrite the expansion as an intentional join".to_owned())
    }
}

pub fn array_slice_in_comparison(checker: &mut Checker) {
    let locator = checker.locator();
    let source = checker.source();
    let direct_operand_diagnostics = [
        ExpansionContext::StringTestOperand,
        ExpansionContext::RegexOperand,
    ]
    .into_iter()
    .flat_map(|context| checker.facts().words().expansion_word_facts(context))
    .filter(|fact| !fact.is_nested_word_command())
    .filter(|fact| fact.has_direct_all_elements_array_expansion_in_source(locator))
    .map(|fact| {
        (
            fact.span(),
            intentional_join_fix(fact.direct_all_elements_array_expansion_spans(), source),
        )
    })
    .collect::<Vec<_>>();

    let risky_pattern_words = checker
        .facts()
        .words()
        .expansion_word_facts(ExpansionContext::ConditionalPattern)
        .filter(|fact| !fact.is_nested_word_command())
        .filter(|fact| fact.command_substitution_spans().is_empty())
        .filter(|fact| !fact.is_pure_positional_at_splat())
        .filter(|fact| fact.has_direct_all_elements_array_expansion_in_source(locator))
        .map(|fact| RiskyPatternWord {
            span: fact.span(),
            expansion_spans: fact.direct_all_elements_array_expansion_spans(),
        })
        .collect::<Vec<_>>();

    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| fact.conditional())
        .flat_map(|conditional| conditional.nodes().iter())
        .flat_map(|node| conditional_pattern_spans(node, &risky_pattern_words, source))
        .chain(direct_operand_diagnostics)
        .collect::<Vec<_>>();

    for (span, fix) in diagnostics {
        let diagnostic = Diagnostic::new(ArraySliceInComparison, span);
        if let Some(fix) = fix {
            checker.report_diagnostic_dedup(diagnostic.with_fix(fix));
        } else {
            checker.report_diagnostic_dedup(diagnostic);
        }
    }
}

#[derive(Clone, Copy)]
struct RiskyPatternWord<'a> {
    span: Span,
    expansion_spans: &'a [Span],
}

fn conditional_pattern_spans(
    fact: &ConditionalNodeFact<'_>,
    risky_words: &[RiskyPatternWord<'_>],
    source: &str,
) -> Vec<(Span, Option<Fix>)> {
    match fact {
        ConditionalNodeFact::Binary(binary)
            if binary.operator_family() != ConditionalOperatorFamily::Logical =>
        {
            [
                pattern_span_if_risky(binary.left().expression().span(), risky_words, source),
                pattern_span_if_risky(binary.right().expression().span(), risky_words, source),
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

fn pattern_span_if_risky(
    span: Span,
    risky_words: &[RiskyPatternWord<'_>],
    source: &str,
) -> Option<(Span, Option<Fix>)> {
    let expansion_spans = risky_words
        .iter()
        .filter(|word| span_contains(span, word.span))
        .flat_map(|word| word.expansion_spans.iter().copied())
        .collect::<Vec<_>>();

    (!expansion_spans.is_empty()).then(|| (span, intentional_join_fix(&expansion_spans, source)))
}

fn span_contains(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && outer.end.offset >= inner.end.offset
}

fn intentional_join_fix(expansion_spans: &[Span], source: &str) -> Option<Fix> {
    expansion_spans
        .iter()
        .map(|span| {
            intentional_join_expansion(span.slice(source))
                .map(|replacement| Edit::replacement(replacement, *span))
        })
        .collect::<Option<Vec<_>>>()
        .filter(|edits| !edits.is_empty())
        .map(Fix::unsafe_edits)
}

fn intentional_join_expansion(raw: &str) -> Option<String> {
    if raw == "$@" {
        return Some("$*".to_owned());
    }

    if let Some(rest) = raw.strip_prefix("$@[*]") {
        return Some(format!("$*{rest}"));
    }

    if raw.starts_with("${@") {
        return Some(raw.replacen("${@", "${*", 1));
    }

    if raw.contains("[@]") {
        return Some(raw.replacen("[@]", "[*]", 1));
    }

    None
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect, assert_diagnostics_diff};

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

    #[test]
    fn ignores_zsh_indexed_and_sliced_positional_parameter_tests() {
        let source = "\
#!/bin/zsh
set -- alpha --scope tail
_i=2
if [[ \"${@[_i]}\" == --scope ]]; then :; fi
if [[ \"$@[_i]\" == --scope ]]; then :; fi
if [[ \"${@[2,-1]}\" == \"--scope tail\" ]]; then :; fi
if [[ \"${@[5,-1]:-fallback}\" == fallback ]]; then :; fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArraySliceInComparison).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_zsh_positional_star_selector_tests() {
        let source = "\
#!/bin/zsh
set -- alpha --scope tail
if [[ \"${@[*]}\" == \"alpha --scope tail\" ]]; then :; fi
if [[ \"$@[*]\" == \"alpha --scope tail\" ]]; then :; fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArraySliceInComparison).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["\"${@[*]}\"", "\"$@[*]\""]
        );
    }

    #[test]
    fn applies_unsafe_fix_to_all_elements_expansions_in_double_bracket_tests() {
        let source = "\
#!/bin/bash
if [[ \"${sel[@]:0:4}\" == \"HELP\" ]]; then :; fi
if [[ -n \"$@\" ]]; then :; fi
if [[ x == *${arr[@]}* ]]; then :; fi
if [[ \"_$@:${arr[@]}\" = x ]]; then :; fi
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::ArraySliceInComparison),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 4);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
if [[ \"${sel[*]:0:4}\" == \"HELP\" ]]; then :; fi
if [[ -n \"$*\" ]]; then :; fi
if [[ x == *${arr[*]}* ]]; then :; fi
if [[ \"_$*:${arr[*]}\" = x ]]; then :; fi
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn safe_fix_mode_leaves_all_elements_expansions_unchanged() {
        let source = "#!/bin/bash\nif [[ -n \"$@\" ]]; then :; fi\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::ArraySliceInComparison),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixed_diagnostics.len(), 1);
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C112.sh").as_path(),
            &LinterSettings::for_rule(Rule::ArraySliceInComparison),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C112_fix_C112.sh", result);
        Ok(())
    }
}
