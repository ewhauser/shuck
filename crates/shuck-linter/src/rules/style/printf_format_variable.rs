use shuck_ast::{Command, Word};

use crate::{Checker, Rule, Violation};

use super::syntax::{static_word_text, walk_commands};

pub struct PrintfFormatVariable;

impl Violation for PrintfFormatVariable {
    fn rule() -> Rule {
        Rule::PrintfFormatVariable
    }

    fn message(&self) -> String {
        "keep `printf` format strings literal instead of expanding them from variables".to_owned()
    }
}

pub fn printf_format_variable(checker: &mut Checker) {
    let source = checker.source();
    let mut spans = Vec::new();

    walk_commands(&checker.ast().commands, &mut |command| {
        let Command::Simple(command) = command else {
            return;
        };

        if static_word_text(&command.name, source).as_deref() != Some("printf") {
            return;
        }

        let Some(format_word) = printf_format_word(&command.args, source) else {
            return;
        };

        if static_word_text(format_word, source).is_none() {
            spans.push(format_word.span);
        }
    });

    for span in spans {
        checker.report(PrintfFormatVariable, span);
    }
}

fn printf_format_word<'a>(args: &'a [Word], source: &str) -> Option<&'a Word> {
    let mut index = 0usize;

    if static_word_text(args.get(index)?, source).as_deref() == Some("--") {
        index += 1;
    }

    if let Some(option) = args
        .get(index)
        .and_then(|word| static_word_text(word, source))
    {
        if option == "-v" {
            index += 2;
        } else if option.starts_with("-v") && option.len() > 2 {
            index += 1;
        }
    }

    args.get(index)
}
