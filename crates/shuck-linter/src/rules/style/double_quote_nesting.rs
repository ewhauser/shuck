use crate::{
    Checker, ExpansionContext, Rule, Violation, word_nested_dynamic_double_quote_spans,
    word_unquoted_scalar_between_double_quoted_segments_spans,
};

pub struct DoubleQuoteNesting;

impl Violation for DoubleQuoteNesting {
    fn rule() -> Rule {
        Rule::DoubleQuoteNesting
    }

    fn message(&self) -> String {
        "a double-quoted expansion is nested between reopened double-quoted text".to_owned()
    }
}

pub fn double_quote_nesting(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .expansion_word_facts(ExpansionContext::CommandName)
        .chain(
            checker
                .facts()
                .expansion_word_facts(ExpansionContext::CommandArgument),
        )
        .chain(
            checker
                .facts()
                .expansion_word_facts(ExpansionContext::AssignmentValue),
        )
        .chain(
            checker
                .facts()
                .expansion_word_facts(ExpansionContext::DeclarationAssignmentValue),
        )
        .filter(|fact| !fact.is_nested_word_command())
        .flat_map(|fact| {
            let mut candidate_spans = fact
                .unquoted_scalar_expansion_spans()
                .iter()
                .copied()
                .filter(|span| !span.slice(source).starts_with("$(("))
                .collect::<Vec<_>>();
            candidate_spans.extend(fact.unquoted_command_substitution_spans().iter().copied());

            word_unquoted_scalar_between_double_quoted_segments_spans(
                fact.word(),
                &candidate_spans,
            )
            .into_iter()
            .chain(word_nested_dynamic_double_quote_spans(fact.word()))
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || DoubleQuoteNesting);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_unquoted_scalar_expansions_between_double_quoted_segments() {
        let source = "\
#!/bin/bash
echo \"$PIDFILE exists, skipping \"$INTERVAL\" run\"
echo \"left \"$v\" right\"
echo \"left \"${v}\" right\"
echo \"left \"$(printf '%s' ok)\" right\"
bash -c \"$pip install \"$(echo -I)\" $pkg\"
x=\"left \"$v\" right\"
value=\"\n-DLZ4_HOME=\"${TERMUX_PREFIX}\"\n-DPROTOBUF_HOME=\"$(printf '%s' proto)\"\n\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DoubleQuoteNesting));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "$INTERVAL",
                "$v",
                "${v}",
                "$(printf '%s' ok)",
                "$(echo -I)",
                "$v",
                "${TERMUX_PREFIX}",
                "$(printf '%s' proto)"
            ]
        );
    }

    #[test]
    fn ignores_non_nested_or_non_dynamic_patterns() {
        let source = "\
#!/bin/bash
echo \"$v\"
echo \"$v\" \"$w\"
echo \"left \"${arr[@]}\" right\"
echo \"$(printf '%s' \"$x\")\"
echo \" in \"$((B-A))\"ms\"
case \"$line\" in \"status-filtered \"$status\"*) : ;; esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DoubleQuoteNesting));

        assert!(diagnostics.is_empty());
    }
}
