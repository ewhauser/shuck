use shuck_ast::{Command, Word};

use crate::rules::common::query::{self, CommandWalkOptions};
use crate::{Checker, Rule, Violation};

use super::syntax::{static_word_text, word_has_expansion, word_is_double_quoted};

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
    let mut spans = Vec::new();

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| {
            let Command::Simple(command) = command else {
                return;
            };

            if static_word_text(&command.name, source).as_deref() != Some("trap") {
                return;
            }

            let Some(action) = trap_action_word(&command.args, source) else {
                return;
            };

            if word_is_double_quoted(indexer, action) && word_has_expansion(action) {
                spans.push(action.span);
            }
        },
    );

    for span in spans {
        checker.report(TrapStringExpansion, span);
    }
}

fn trap_action_word<'a>(args: &'a [Word], source: &str) -> Option<&'a Word> {
    let mut start = 0usize;

    if let Some(first) = args.first().and_then(|word| static_word_text(word, source)) {
        match first.as_str() {
            "-p" | "-l" => return None,
            "--" => start = 1,
            _ => {}
        }
    }

    let action = args.get(start)?;
    args.get(start + 1)?;
    Some(action)
}
