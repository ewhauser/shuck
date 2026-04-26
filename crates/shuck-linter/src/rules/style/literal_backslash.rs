use crate::{Checker, ExpansionContext, Rule, Violation};

pub struct LiteralBackslash;

impl Violation for LiteralBackslash {
    fn rule() -> Rule {
        Rule::LiteralBackslash
    }

    fn message(&self) -> String {
        "a backslash before a normal letter is literal".to_owned()
    }
}

pub fn literal_backslash(checker: &mut Checker) {
    let source = checker.source();
    let facts = checker.facts();
    let spans = checker
        .facts()
        .word_facts()
        .iter()
        .filter(|fact| is_relevant_word_context(fact.expansion_context()))
        .filter(|fact| !fact.is_nested_word_command())
        .filter(|fact| !is_command_name_word(facts, *fact))
        .filter(|fact| !is_unalias_argument(facts, *fact))
        .filter_map(|fact| fact.standalone_literal_backslash_span(source))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || LiteralBackslash);
}

fn is_relevant_word_context(context: Option<ExpansionContext>) -> bool {
    matches!(
        context,
        Some(
            ExpansionContext::CommandArgument
                | ExpansionContext::ForList
                | ExpansionContext::SelectList
        )
    )
}

fn is_command_name_word<'a>(
    facts: &'a crate::facts::LinterFacts<'a>,
    fact: crate::facts::WordOccurrenceRef<'_, 'a>,
) -> bool {
    facts
        .command(fact.command_id())
        .arena_body_name_word(facts.source())
        .is_some_and(|word| word.span() == fact.span())
}

fn is_unalias_argument<'a>(
    facts: &'a crate::facts::LinterFacts<'a>,
    fact: crate::facts::WordOccurrenceRef<'_, 'a>,
) -> bool {
    fact.expansion_context() == Some(ExpansionContext::CommandArgument)
        && facts
            .command(fact.command_id())
            .effective_name_is("unalias")
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_literal_backslashes_before_normal_letters() {
        let source = "\
#!/bin/sh
echo \\q
unalias \\R
\\command \\ls -ld file
printf '%s\\n' \\q
echo \\command
echo foo\\xbar
foo=bar\\w
case x in foo\\q) : ;; esac
cat < foo\\q
echo \\n
echo \\Q
echo \"\\q\"
echo '\\q'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBackslash));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["", ""]
        );
    }
}
