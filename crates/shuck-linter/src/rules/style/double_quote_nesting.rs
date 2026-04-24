use crate::{Checker, ExpansionContext, Rule, Violation};

pub struct DoubleQuoteNesting;

impl Violation for DoubleQuoteNesting {
    fn rule() -> Rule {
        Rule::DoubleQuoteNesting
    }

    fn message(&self) -> String {
        "a double-quoted expansion is nested between reopened double-quoted text".to_owned()
    }
}

fn span_is_strictly_inside(span: &shuck_ast::Span, host: &shuck_ast::Span) -> bool {
    host.start.offset <= span.start.offset
        && span.end.offset <= host.end.offset
        && span.start.offset != host.start.offset
        && span.end.offset != host.end.offset
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
        .flat_map(|fact| {
            let mut candidate_spans = fact
                .unquoted_scalar_expansion_spans()
                .iter()
                .copied()
                .filter(|span| !span.slice(source).starts_with("$(("))
                .collect::<Vec<_>>();
            candidate_spans.extend(
                fact.unquoted_dollar_paren_command_substitution_spans()
                    .iter()
                    .copied(),
            );

            let host_substitution_spans = fact.command_substitution_spans();

            fact.unquoted_scalar_between_double_quoted_segments_spans(&candidate_spans)
                .into_iter()
                .chain(fact.nested_dynamic_double_quote_spans())
                .filter(|span| {
                    !host_substitution_spans
                        .iter()
                        .any(|host| span_is_strictly_inside(span, host))
                })
        })
        .chain(
            checker
                .facts()
                .comment_double_quote_nesting_spans()
                .iter()
                .copied(),
        )
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
    fn ignores_legacy_backticks_between_double_quoted_segments() {
        let source = "\
#!/bin/bash
echo \"left \"`printf '%s' value`\" right\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DoubleQuoteNesting));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_oracle_comment_quote_nesting_pattern() {
        let source = "\
#!/bin/sh
# script's $0 value, followed by \"$@\".
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DoubleQuoteNesting));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$@"]
        );
    }

    #[test]
    fn ignores_heredoc_payload_comment_like_quote_nesting_pattern() {
        let source = "\
#!/bin/sh
cat <<'EOF'
# script's $0 value, followed by \"$@\".
EOF
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DoubleQuoteNesting));

        assert!(diagnostics.is_empty());
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

    #[test]
    fn reports_reopened_quotes_inside_unquoted_command_substitution_bodies() {
        let source = "\
#!/bin/bash
now=$(grep -aE '^5' \"$LISTFILE\" | sed -n \"\"$rule_link\"p\" | awk '{print $2}')
server=$(grep -aE '^3|^4' \"$LISTFILE\" | sed -n \"\"$server_link\"p\" | awk '{print $3}')
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DoubleQuoteNesting));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$rule_link", "$server_link"]
        );
    }

    #[test]
    fn ignores_nested_command_arguments_that_are_fully_quoted_inside_quoted_substitutions() {
        let source = "\
#!/bin/bash
files+=(\"$(basename \"${file%.desktop}\")\")
candidates+=(\"$(echo \"$line\" | cut -d' ' -f2-)\")
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DoubleQuoteNesting));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            Vec::<&str>::new()
        );
    }
}
