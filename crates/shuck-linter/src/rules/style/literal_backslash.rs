use shuck_ast::{Span, Word, WordPart};

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
        .filter(|fact| !is_command_name_word(facts, fact))
        .filter(|fact| !is_unalias_argument(facts, fact))
        .filter_map(|fact| standalone_literal_backslash_span(fact.word(), source))
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

fn standalone_literal_backslash_span(word: &Word, source: &str) -> Option<Span> {
    let [part] = word.parts.as_slice() else {
        return None;
    };
    if !matches!(part.kind, WordPart::Literal(_)) {
        return None;
    }

    let text = word.span.slice(source);
    let bytes = text.as_bytes();
    if bytes.len() != 2 || bytes[0] != b'\\' {
        return None;
    }

    let target = bytes[1];
    if !target.is_ascii_lowercase() || matches!(target, b'n' | b'r' | b't') {
        return None;
    }

    Some(Span::from_positions(word.span.start, word.span.start))
}

fn is_command_name_word<'a>(
    facts: &'a crate::facts::LinterFacts<'a>,
    fact: &crate::facts::WordFact<'a>,
) -> bool {
    facts
        .command(fact.command_id())
        .body_name_word()
        .is_some_and(|word| word.span == fact.span())
}

fn is_unalias_argument<'a>(
    facts: &'a crate::facts::LinterFacts<'a>,
    fact: &crate::facts::WordFact<'a>,
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
