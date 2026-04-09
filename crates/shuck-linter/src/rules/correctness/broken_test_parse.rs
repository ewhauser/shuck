use crate::{Checker, Rule, Violation, static_word_text};

pub struct BrokenTestParse;

impl Violation for BrokenTestParse {
    fn rule() -> Rule {
        Rule::BrokenTestParse
    }

    fn message(&self) -> String {
        "`[` test expression is malformed".to_owned()
    }
}

pub fn broken_test_parse(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.static_utility_name_is("["))
        .filter(|fact| {
            fact.body_args()
                .last()
                .and_then(|word| static_word_text(word, checker.source()))
                .as_deref()
                != Some("]")
        })
        .map(|fact| fact.body_name_word().map_or(fact.span(), |word| word.span))
        .collect::<Vec<_>>();

    checker.report_all(spans, || BrokenTestParse);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_malformed_bracket_tests() {
        let source = "#!/bin/sh\nif [ x = y; then :; fi\n[ foo\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BrokenTestParse));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), "[");
        assert_eq!(diagnostics[1].span.slice(source), "[");
    }

    #[test]
    fn ignores_well_formed_test_commands() {
        let source = "#!/bin/sh\nif [ x = y ]; then :; fi\ntest x = y\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BrokenTestParse));

        assert!(diagnostics.is_empty());
    }
}
