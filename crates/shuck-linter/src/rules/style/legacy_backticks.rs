use crate::rules::common::query::{self, CommandWalkOptions};
use crate::rules::common::span;
use crate::{Checker, Rule, Violation};

pub struct LegacyBackticks;

impl Violation for LegacyBackticks {
    fn rule() -> Rule {
        Rule::LegacyBackticks
    }

    fn message(&self) -> String {
        "prefer `$(...)` over legacy backtick substitution".to_owned()
    }
}

pub fn legacy_backticks(checker: &mut Checker) {
    let source = checker.source();

    query::walk_words(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |word| {
            for span in span::backtick_fragment_spans(word, source) {
                checker.report_dedup(LegacyBackticks, span);
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_each_backtick_fragment() {
        let source = "echo \"prefix `date` suffix `uname`\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LegacyBackticks));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["`date`", "`uname`"]
        );
    }
}
