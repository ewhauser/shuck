use shuck_ast::{Word, WordPart};

use crate::{Checker, Rule, Violation};

use super::syntax::{visit_argument_words, walk_commands};

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
    let mut spans = Vec::new();

    walk_commands(&checker.ast().commands, &mut |command| {
        visit_argument_words(command, |word| {
            if word_has_unquoted_array_expansion(word, source) {
                spans.push(word.span);
            }
        });
    });

    spans.sort_unstable_by_key(|span| (span.start.offset, span.end.offset));
    spans.dedup();

    for span in spans {
        checker.report(UnquotedArrayExpansion, span);
    }
}

fn word_has_unquoted_array_expansion(word: &Word, source: &str) -> bool {
    if word.quoted {
        return false;
    }

    word.parts.iter().any(|part| match part {
        WordPart::ArrayAccess { index, .. } => {
            let index = index.slice(source);
            index == "@" || index == "*"
        }
        WordPart::ArraySlice { .. } => true,
        _ => false,
    })
}
