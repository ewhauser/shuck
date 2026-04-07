use crate::rules::common::span;
use crate::rules::common::word::classify_word;
use crate::rules::common::{
    expansion::ExpansionContext,
    query::{self, CommandWalkOptions},
};
use crate::{Checker, Rule, Violation};

use super::syntax::word_is_double_quoted;

pub struct TrapStringExpansion;

impl Violation for TrapStringExpansion {
    fn rule() -> Rule {
        Rule::TrapStringExpansion
    }

    fn message(&self) -> String {
        "double-quoted trap handlers expand variables when the trap is set".to_owned()
    }
}

pub fn trap_string_expansion(checker: &mut Checker) {
    let source = checker.source();
    let indexer = checker.indexer();

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| {
            query::visit_expansion_words(command, source, &mut |word, context| {
                if context != ExpansionContext::TrapAction {
                    return;
                }

                if word_is_double_quoted(indexer, word) && classify_word(word, source).is_expanded()
                {
                    for span in span::expansion_part_spans(word) {
                        checker.report_dedup(TrapStringExpansion, span);
                    }
                }
            });
        },
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_each_expansion_inside_the_trap_action() {
        let source = "trap \"echo $x $(date) ${y}\" EXIT\n";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::TrapStringExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$x", "$(date)", "${y}"]
        );
    }

    #[test]
    fn ignores_trap_listing_modes() {
        let source = "trap -p EXIT\ntrap -l TERM\n";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::TrapStringExpansion));

        assert!(diagnostics.is_empty());
    }
}
