use crate::{
    Checker, ExpansionContext, Rule, Violation,
    word_unquoted_literal_between_double_quoted_segments_spans,
};

pub struct MixedQuoteWord;

impl Violation for MixedQuoteWord {
    fn rule() -> Rule {
        Rule::MixedQuoteWord
    }

    fn message(&self) -> String {
        "avoid mixing bare fragments between reopened double-quoted text".to_owned()
    }
}

pub fn mixed_quote_word(checker: &mut Checker) {
    let source = checker.source();
    let facts = checker.facts();
    let spans = [
        ExpansionContext::CommandName,
        ExpansionContext::CommandArgument,
        ExpansionContext::AssignmentValue,
        ExpansionContext::DeclarationAssignmentValue,
        ExpansionContext::StringTestOperand,
        ExpansionContext::CasePattern,
    ]
    .into_iter()
    .flat_map(|context| facts.expansion_word_facts(context))
    .chain(facts.case_subject_facts())
    .filter(|fact| !fact.is_nested_word_command())
    .flat_map(|fact| {
        word_unquoted_literal_between_double_quoted_segments_spans(fact.word(), source)
    })
    .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || MixedQuoteWord);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_bare_fragments_between_reopened_double_quotes() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"left \"middle\" right\" \"foo\"-\"bar\"
name=\"foo\"bar\"baz\"
declare local_name=\"foo\"bar\"baz\"
if [ \"foo\"bar\"baz\" = x ]; then :; fi
case \"foo\"bar\"baz\" in x) : ;; esac
case x in \"foo\"bar\"baz\") : ;; esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MixedQuoteWord));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["middle", "-", "bar", "bar", "bar", "bar"]
        );
    }

    #[test]
    fn ignores_single_quotes_dynamic_middles_and_separator_literals() {
        let source = "\
#!/bin/bash
printf '%s\\n' 'foo'bar'baz'
printf '%s\\n' \"foo\"${bar}\"baz\" \"foo\"$(printf '%s' x)\"baz\"
printf '%s\\n' \"$left\"-\"$right\" \"$left\".\"$right\" \"$left\"@\"$right\"
printf '%s\\n' \"foo\"/\"bar\" \"foo\"=\"bar\" \"foo\":\"bar\" \"foo\"?\"bar\"
if [[ x =~ \"foo\"bar\"baz\" ]]; then :; fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MixedQuoteWord));

        assert!(diagnostics.is_empty());
    }
}
