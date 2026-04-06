use crate::rules::common::query::{self, CommandWalkOptions};
use crate::rules::common::span;
use crate::rules::common::word::classify_word;
use crate::{Checker, Rule, Violation};

use super::syntax::visit_argument_words;

pub struct UnquotedCommandSubstitution;

impl Violation for UnquotedCommandSubstitution {
    fn rule() -> Rule {
        Rule::UnquotedCommandSubstitution
    }

    fn message(&self) -> String {
        "quote command substitutions in arguments to avoid word splitting".to_owned()
    }
}

pub fn unquoted_command_substitution(checker: &mut Checker) {
    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: false,
        },
        &mut |command, _| {
            visit_argument_words(command, |word| {
                let classification = classify_word(word, checker.source());
                if classification.has_command_substitution() {
                    for span in span::unquoted_command_substitution_part_spans(word) {
                        checker.report_dedup(UnquotedCommandSubstitution, span);
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
    fn anchors_on_inner_command_substitution_spans() {
        let source = "printf '%s\\n' prefix$(date)suffix $(uname)\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$(date)", "$(uname)"]
        );
    }
}
