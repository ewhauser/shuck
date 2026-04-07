use shuck_ast::Command;

use crate::rules::common::query::{self, CommandWalkOptions};
use crate::rules::common::word::static_word_text;
use crate::{Checker, Rule, Violation};

pub struct ReadWithoutRaw;

impl Violation for ReadWithoutRaw {
    fn rule() -> Rule {
        Rule::ReadWithoutRaw
    }

    fn message(&self) -> String {
        "use `read -r` to keep backslashes literal".to_owned()
    }
}

pub fn read_without_raw(checker: &mut Checker) {
    let source = checker.source();
    let mut spans = Vec::new();

    query::walk_commands(
        &checker.ast().body,
        CommandWalkOptions {
            descend_nested_word_commands: false,
        },
        &mut |visit| {
            let command = visit.command;
            let Command::Simple(command) = command else {
                return;
            };

            if static_word_text(&command.name, source).as_deref() != Some("read") {
                return;
            }

            if !read_uses_raw_input(&command.args, source) {
                spans.push(command.name.span);
            }
        },
    );

    for span in spans {
        checker.report(ReadWithoutRaw, span);
    }
}

fn read_uses_raw_input(args: &[shuck_ast::Word], source: &str) -> bool {
    let mut index = 0usize;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };

        if text == "--" {
            break;
        }

        if !text.starts_with('-') || text == "-" {
            break;
        }

        let mut chars = text[1..].chars().peekable();
        while let Some(flag) = chars.next() {
            if flag == 'r' {
                return true;
            }

            if option_takes_argument(flag) {
                if chars.peek().is_none() {
                    index += 1;
                }
                break;
            }
        }

        index += 1;
    }

    false
}

fn option_takes_argument(flag: char) -> bool {
    matches!(flag, 'a' | 'd' | 'i' | 'n' | 'N' | 'p' | 't' | 'u')
}
