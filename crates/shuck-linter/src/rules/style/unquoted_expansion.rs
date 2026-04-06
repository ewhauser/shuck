use shuck_ast::{Word, WordPart};

use crate::{Checker, Rule, Violation};

use super::syntax::{visit_argument_words, walk_commands};

pub struct UnquotedExpansion;

impl Violation for UnquotedExpansion {
    fn rule() -> Rule {
        Rule::UnquotedExpansion
    }

    fn message(&self) -> String {
        "quote parameter expansions in arguments to avoid word splitting and globbing".to_owned()
    }
}

pub fn unquoted_expansion(checker: &mut Checker) {
    let source = checker.source();
    let mut spans = Vec::new();

    walk_commands(&checker.ast().commands, &mut |command| {
        visit_argument_words(command, |word| {
            if word_has_unquoted_scalar_expansion(word, source) {
                spans.push(word.span);
            }
        });
    });

    spans.sort_unstable_by_key(|span| (span.start.offset, span.end.offset));
    spans.dedup();

    for span in spans {
        checker.report(UnquotedExpansion, span);
    }
}

fn word_has_unquoted_scalar_expansion(word: &Word, source: &str) -> bool {
    if word.quoted {
        return false;
    }

    word.parts.iter().any(|part| match part {
        WordPart::Variable(_)
        | WordPart::ParameterExpansion { .. }
        | WordPart::Substring { .. }
        | WordPart::IndirectExpansion { .. }
        | WordPart::PrefixMatch(_)
        | WordPart::Transformation { .. } => true,
        WordPart::ArrayAccess { index, .. } => {
            let index = index.slice(source);
            index != "@" && index != "*"
        }
        _ => false,
    })
}
