use crate::rules::common::query::{self, CommandWalkOptions};
use crate::rules::common::span;
use crate::rules::common::word::classify_word;
use crate::{Checker, Rule, Violation};

use super::syntax::visit_argument_words;

pub struct UnquotedArrayExpansion;

impl Violation for UnquotedArrayExpansion {
    fn rule() -> Rule {
        Rule::UnquotedArrayExpansion
    }

    fn message(&self) -> String {
        "quote array expansions to preserve element boundaries".to_owned()
    }
}

pub fn unquoted_array_expansion(checker: &mut Checker) {
    let source = checker.source();

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: false,
        },
        &mut |command, _| {
            visit_argument_words(command, |word| {
                let classification = classify_word(word, source);
                if classification.has_array_expansion() {
                    for span in span::unquoted_array_expansion_part_spans(word, source) {
                        checker.report_dedup(UnquotedArrayExpansion, span);
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
    fn anchors_on_inner_array_expansion_spans() {
        let source = "\
#!/bin/bash
printf '%s\\n' prefix${arr[@]}suffix ${arr[0]} ${names[*]}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedArrayExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[@]}", "${names[*]}"]
        );
    }
}
